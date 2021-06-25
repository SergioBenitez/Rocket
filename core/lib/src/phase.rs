use state::Container;
use figment::Figment;

use crate::websocket::broker::Broker;
use crate::{Catcher, Config, Rocket, Route, Shutdown};
use crate::router::Router;
use crate::fairing::Fairings;

mod private {
    pub trait Sealed {  }
}

#[doc(hidden)]
pub trait Stateful: private::Sealed {
    fn into_state(self) -> State;
    fn as_state_ref(&self) -> StateRef<'_>;
}

/// A marker trait for Rocket's launch phases.
///
/// This treat is implemented by the three phase marker types: [`Build`],
/// [`Ignite`], and [`Orbit`], representing the three phases to launch an
/// instance of [`Rocket`]. This trait is _sealed_ and cannot be implemented
/// outside of Rocket.
///
/// For a description of the three phases, see [`Rocket#phases`].
pub trait Phase: private::Sealed {
    #[doc(hidden)]
    type State: std::fmt::Debug + Stateful + Sync + Send + Unpin;
}

macro_rules! phase {
    ($(#[$o:meta])* $P:ident ($(#[$i:meta])* $S:ident) { $($fields:tt)* }) => (
        $(#[$o])*
        pub enum $P { }

        impl Phase for $P {
            #[doc(hidden)]
            type State = $S;
        }

        $(#[$i])*
        #[doc(hidden)]
        pub struct $S {
            $($fields)*
        }

        impl Stateful for $S {
            fn into_state(self) -> State { State::$P(self) }
            fn as_state_ref(&self) -> StateRef<'_> { StateRef::$P(self) }
        }

        #[doc(hidden)]
        impl From<$S> for Rocket<$P> {
            fn from(s: $S) -> Self { Rocket(s) }
        }

        impl private::Sealed for $P {}

        impl private::Sealed for $S {}
    )
}

macro_rules! phases {
    ($($(#[$o:meta])* $P:ident ($(#[$i:meta])* $S:ident) { $($fields:tt)* })*) => (
        #[doc(hidden)]
        pub enum State { $($P($S)),* }

        #[doc(hidden)]
        pub enum StateRef<'a> { $($P(&'a $S)),* }

        $(phase!($(#[$o])* $P ($(#[$i])* $S) { $($fields)* });)*
    )
}

phases! {
    /// The initial launch [`Phase`].
    ///
    /// An instance of `Rocket` in this phase is typed as [`Rocket<Build>`] and
    /// represents a transient, in-progress build. See [`Rocket#build`] for
    /// details.
    Build (#[derive(Default, Debug)] Building) {
        pub(crate) routes: Vec<Route>,
        pub(crate) catchers: Vec<Catcher>,
        pub(crate) fairings: Fairings,
        pub(crate) figment: Figment,
        //pub(crate) broker: Broker,
        pub(crate) state: Container![Send + Sync],
    }

    /// The second launch [`Phase`]: post-build but pre-orbit.
    ///
    /// An instance of `Rocket` in this phase is typed as [`Rocket<Ignite>`] and
    /// represents a fully built and finalized application server ready for
    /// launch into orbit. See [`Rocket#ignite`] for full details.
    Ignite (#[derive(Debug)] Igniting) {
        pub(crate) router: Router,
        pub(crate) fairings: Fairings,
        pub(crate) figment: Figment,
        pub(crate) config: Config,
        pub(crate) broker: Broker,
        pub(crate) state: Container![Send + Sync],
        pub(crate) shutdown: Shutdown,
    }

    /// The final launch [`Phase`].
    ///
    /// An instance of `Rocket` in this phase is typed as [`Rocket<Orbit>`] and
    /// represents a running application. See [`Rocket#orbit`] for full details.
    Orbit (#[derive(Debug)] Orbiting) {
        pub(crate) router: Router,
        pub(crate) fairings: Fairings,
        pub(crate) figment: Figment,
        pub(crate) config: Config,
        pub(crate) broker: Broker,
        pub(crate) state: Container![Send + Sync],
        pub(crate) shutdown: Shutdown,
    }
}
