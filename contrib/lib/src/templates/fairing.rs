use std::error::Error;

use crate::templates::{DEFAULT_TEMPLATE_DIR, Context, Engines};

use rocket::{Rocket, Build, Orbit};
use rocket::fairing::{self, Fairing, Info, Kind};

pub(crate) use self::context::ContextManager;

type Callback = Box<dyn Fn(&mut Engines) -> Result<(), Box<dyn Error>>+ Send + Sync + 'static>;

#[cfg(not(debug_assertions))]
mod context {
    use std::ops::Deref;
    use crate::templates::Context;

    /// Wraps a Context. With `cfg(debug_assertions)` active, this structure
    /// additionally provides a method to reload the context at runtime.
    pub(crate) struct ContextManager(Context);

    impl ContextManager {
        pub fn new(ctxt: Context) -> ContextManager {
            ContextManager(ctxt)
        }

        pub fn context<'a>(&'a self) -> impl Deref<Target=Context> + 'a {
            &self.0
        }

        pub fn is_reloading(&self) -> bool {
            false
        }
    }
}

#[cfg(debug_assertions)]
mod context {
    use std::ops::{Deref, DerefMut};
    use std::sync::{RwLock, Mutex};
    use std::sync::mpsc::{channel, Receiver};

    use notify::{raw_watcher, RawEvent, RecommendedWatcher, RecursiveMode, Watcher};

    use super::{Callback, Context};

    /// Wraps a Context. With `cfg(debug_assertions)` active, this structure
    /// additionally provides a method to reload the context at runtime.
    pub(crate) struct ContextManager {
        /// The current template context, inside an RwLock so it can be updated.
        context: RwLock<Context>,
        /// A filesystem watcher and the receive queue for its events.
        watcher: Option<(RecommendedWatcher, Mutex<Receiver<RawEvent>>)>,
    }

    impl ContextManager {
        pub fn new(ctxt: Context) -> ContextManager {
            let (tx, rx) = channel();
            let watcher = raw_watcher(tx).and_then(|mut watcher| {
                watcher.watch(ctxt.root.canonicalize()?, RecursiveMode::Recursive)?;
                Ok(watcher)
            });

            let watcher = match watcher {
                Ok(watcher) => Some((watcher, Mutex::new(rx))),
                Err(error) => {
                    warn_span!("Failed to enable live template reloading", %error).in_scope(|| {
                        debug!(reload_error = ?error);
                        warn!("Live template reloading is unavailable.");
                    });
                    None
                }
            };

            ContextManager { watcher, context: RwLock::new(ctxt), }
        }

        pub fn context(&self) -> impl Deref<Target=Context> + '_ {
            self.context.read().unwrap()
        }

        pub fn is_reloading(&self) -> bool {
            self.watcher.is_some()
        }

        fn context_mut(&self) -> impl DerefMut<Target=Context> + '_ {
            self.context.write().unwrap()
        }

        /// Checks whether any template files have changed on disk. If there
        /// have been changes since the last reload, all templates are
        /// reinitialized from disk and the user's customization callback is run
        /// again.
        pub fn reload_if_needed(&self, callback: &Callback) {
            let templates_changes = self.watcher.as_ref()
                .map(|(_, rx)| rx.lock().expect("fsevents lock").try_iter().count() > 0);

            if let Some(true) = templates_changes {
                let span = info_span!("Change detected: reloading templates.");
                let _e = span.enter();
                let root = self.context().root.clone();
                if let Some(mut new_ctxt) = Context::initialize(&root) {
                    match callback(&mut new_ctxt.engines) {
                        Ok(()) => {
                            *self.context_mut() = new_ctxt;
                            debug!("reloaded!");
                        }
                        Err(error) => {
                            warn!(%error, "The template customization callback returned an error");
                            warn!("The existing templates will remain active.");
                        }
                    }
                } else {
                    warn!("An error occurred while reloading templates.");
                    warn!("The existing templates will remain active.");
                };
            }
        }
    }
}

/// The TemplateFairing initializes the template system on attach, running
/// custom_callback after templates have been loaded. In debug mode, the fairing
/// checks for modifications to templates before every request and reloads them
/// if necessary.
pub struct TemplateFairing {
    /// The user-provided customization callback, allowing the use of
    /// functionality specific to individual template engines. In debug mode,
    /// this callback might be run multiple times as templates are reloaded.
    pub callback: Callback,
}

#[rocket::async_trait]
impl Fairing for TemplateFairing {
    fn info(&self) -> Info {
        let kind = Kind::Ignite | Kind::Liftoff;
        #[cfg(debug_assertions)] let kind = kind | Kind::Request;

        Info { kind, name: "Templating" }
    }

    /// Initializes the template context. Templates will be searched for in the
    /// `template_dir` config variable or the default ([DEFAULT_TEMPLATE_DIR]).
    /// The user's callback, if any was supplied, is called to customize the
    /// template engines. In debug mode, the `ContextManager::new` method
    /// initializes a directory watcher for auto-reloading of templates.
    async fn on_ignite(&self, rocket: Rocket<Build>) -> fairing::Result {
        use rocket::figment::value::magic::RelativePathBuf;

        let configured_dir = rocket.figment()
            .extract_inner::<RelativePathBuf>("template_dir")
            .map(|path| path.relative());

        let path = match configured_dir {
            Ok(dir) => dir,
            Err(e) if e.missing() => DEFAULT_TEMPLATE_DIR.into(),
            Err(e) => {
                rocket::config::pretty_print_error(e);
                return Err(rocket);
            }
        };

        match Context::initialize(&path) {
            Some(mut ctxt) => {
                match (self.callback)(&mut ctxt.engines) {
                    Ok(()) => Ok(rocket.manage(ContextManager::new(ctxt))),
                    Err(error) => {
                        error!(%error, "The template customization callback returned an error");
                        Err(rocket)
                    }
                }
            }
            None => {
                error!("Launch will be aborted due to failed template initialization.");
                Err(rocket)
            }
        }
    }

    async fn on_liftoff(&self, rocket: &Rocket<Orbit>) {
        use rocket::{figment::Source, trace::PaintExt, yansi::Paint};

        let cm = rocket.state::<ContextManager>()
            .expect("Template ContextManager registered in on_ignite");

        let span = info_span!("templating", "{}{}:", Paint::emoji("📐 "), Paint::magenta("Templating"));
        let _e = span.enter();
        info!(directory = %Paint::white(Source::from(&*cm.context().root)));
        info!(engines = ?Paint::white(Engines::ENABLED_EXTENSIONS));
    }

    #[cfg(debug_assertions)]
    async fn on_request(&self, req: &mut rocket::Request<'_>, _data: &mut rocket::Data) {
        let cm = req.rocket().state::<ContextManager>()
            .expect("Template ContextManager registered in on_ignite");

        cm.reload_if_needed(&self.callback);
    }

}
