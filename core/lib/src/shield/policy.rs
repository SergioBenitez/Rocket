//! Module containing the [`Policy`] trait and types that implement it.

use std::fmt;
use std::borrow::Cow;

use indexmap::IndexMap;
use rocket_http::{ext::IntoCollection, private::SmallVec};
use time::Duration;

use crate::http::{Header, uri::Absolute, uncased::{UncasedStr, Uncased}};

/// Trait implemented by security and privacy policy headers.
///
/// Types that implement this trait can be [`enable()`]d and [`disable()`]d on
/// instances of [`Shield`].
///
/// [`Shield`]: crate::shield::Shield
/// [`enable()`]: crate::shield::Shield::enable()
/// [`disable()`]: crate::shield::Shield::disable()
pub trait Policy: Default + Send + Sync + 'static {
    /// The actual name of the HTTP header.
    ///
    /// This name must uniquely identify the header as it is used to determine
    /// whether two implementations of `Policy` are for the same header. Use the
    /// real HTTP header's name.
    ///
    /// # Example
    ///
    /// ```rust
    /// # extern crate rocket;
    /// # use rocket::http::Header;
    /// use rocket::shield::Policy;
    ///
    /// #[derive(Default)]
    /// struct MyPolicy;
    ///
    /// impl Policy for MyPolicy {
    ///     const NAME: &'static str = "X-My-Policy";
    /// #   fn header(&self) -> Header<'static> { unimplemented!() }
    /// }
    /// ```
    const NAME: &'static str;

    /// Returns the [`Header`](../../rocket/http/struct.Header.html) to attach
    /// to all outgoing responses.
    ///
    /// # Example
    ///
    /// ```rust
    /// # extern crate rocket;
    /// use rocket::http::Header;
    /// use rocket::shield::Policy;
    ///
    /// #[derive(Default)]
    /// struct MyPolicy;
    ///
    /// impl Policy for MyPolicy {
    /// #   const NAME: &'static str = "X-My-Policy";
    ///     fn header(&self) -> Header<'static> {
    ///         Header::new(Self::NAME, "value-to-enable")
    ///     }
    /// }
    /// ```
    fn header(&self) -> Header<'static>;
}

/// Hack to make `Policy` Object-Safe.
pub(crate) trait SubPolicy: Send + Sync {
    fn name(&self) -> &'static UncasedStr;
    fn header(&self) -> Header<'static>;
}

impl<P: Policy> SubPolicy for P {
    fn name(&self) -> &'static UncasedStr {
        P::NAME.into()
    }

    fn header(&self) -> Header<'static> {
        Policy::header(self)
    }
}

macro_rules! impl_policy {
    ($T:ty, $name:expr) => (
        impl Policy for $T {
            const NAME: &'static str = $name;

            fn header(&self) -> Header<'static> {
                self.into()
            }
        }
    )
}

// Keep this in-sync with the top-level module docs.
impl_policy!(XssFilter, "X-XSS-Protection");
impl_policy!(NoSniff, "X-Content-Type-Options");
impl_policy!(Frame, "X-Frame-Options");
impl_policy!(Hsts, "Strict-Transport-Security");
impl_policy!(ExpectCt, "Expect-CT");
impl_policy!(Referrer, "Referrer-Policy");
impl_policy!(Prefetch, "X-DNS-Prefetch-Control");
impl_policy!(Permission, "Permissions-Policy");

/// The [Referrer-Policy] header: controls the value set by the browser for the
/// [Referer] header.
///
/// Tells the browser if it should send all or part of URL of the current page
/// to the next site the user navigates to via the [Referer] header. This can be
/// important for security as the URL itself might expose sensitive data, such
/// as a hidden file path or personal identifier.
///
/// [Referrer-Policy]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Referrer-Policy
/// [Referer]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Referer
pub enum Referrer {
    /// Omits the `Referer` header (returned by [`Referrer::default()`]).
    NoReferrer,

    /// Omits the `Referer` header on connection downgrade i.e. following HTTP
    /// link from HTTPS site (_Browser default_).
    NoReferrerWhenDowngrade,

    /// Only send the origin of part of the URL, e.g. the origin of
    /// `https://foo.com/bob.html` is `https://foo.com`.
    Origin,

    /// Send full URL for same-origin requests, only send origin part when
    /// replying to [cross-origin] requests.
    ///
    /// [cross-origin]: https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS
    OriginWhenCrossOrigin,

    /// Send full URL for same-origin requests only.
    SameOrigin,

    /// Only send origin part of URL, only send if protocol security level
    /// remains the same e.g. HTTPS to HTTPS.
    StrictOrigin,

    /// Send full URL for same-origin requests. For cross-origin requests, only
    /// send origin part of URL if protocol security level remains the same e.g.
    /// HTTPS to HTTPS.
    StrictOriginWhenCrossOrigin,

    /// Send full URL for same-origin or cross-origin requests. _This will leak
    /// the full URL of TLS protected resources to insecure origins. Use with
    /// caution._
    UnsafeUrl,
 }

/// Defaults to [`Referrer::NoReferrer`]. Tells the browser to omit the
/// `Referer` header.
impl Default for Referrer {
    fn default() -> Referrer {
        Referrer::NoReferrer
    }
}

impl From<&Referrer> for Header<'static> {
    fn from(referrer: &Referrer) -> Self {
        let policy_string = match referrer {
            Referrer::NoReferrer => "no-referrer",
            Referrer::NoReferrerWhenDowngrade => "no-referrer-when-downgrade",
            Referrer::Origin => "origin",
            Referrer::OriginWhenCrossOrigin => "origin-when-cross-origin",
            Referrer::SameOrigin => "same-origin",
            Referrer::StrictOrigin => "strict-origin",
            Referrer::StrictOriginWhenCrossOrigin => "strict-origin-when-cross-origin",
            Referrer::UnsafeUrl => "unsafe-url",
        };

        Header::new(Referrer::NAME, policy_string)
    }
}

/// The [Expect-CT] header: enables reporting and/or enforcement of [Certificate
/// Transparency].
///
/// [Certificate Transparency] can detect and prevent the use of misissued,
/// malicious, or revoked TLS certificates. It solves a variety of problems with
/// public TLS/SSL certificate management and is valuable measure for all public
/// TLS applications.
///
/// If you're just [getting started] with certificate transparency, ensure that
/// your [site is in compliance][getting started] before you enable enforcement
/// with [`ExpectCt::Enforce`] or [`ExpectCt::ReportAndEnforce`]. Failure to do
/// so will result in the browser refusing to communicate with your application.
/// _You have been warned_.
///
/// [Expect-CT]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Expect-CT
/// [Certificate Transparency]: http://www.certificate-transparency.org/what-is-ct
/// [getting started]: http://www.certificate-transparency.org/getting-started
pub enum ExpectCt {
    /// Enforce certificate compliance for the next [`Duration`]. Ensure that
    /// your certificates are in compliance before turning on enforcement.
    /// (_Shield_ default).
    Enforce(Duration),

    /// Report to `Absolute`, but do not enforce, compliance violations for the
    /// next [`Duration`]. Doesn't provide any protection but is a good way make
    /// sure things are working correctly before turning on enforcement in
    /// production.
    Report(Duration, Absolute<'static>),

    /// Enforce compliance and report violations to `Absolute` for the next
    /// [`Duration`].
    ReportAndEnforce(Duration, Absolute<'static>),
}

/// Defaults to [`ExpectCt::Enforce`] with a 30 day duration, enforce CT
/// compliance, see [draft] standard for more.
///
/// [draft]: https://tools.ietf.org/html/draft-ietf-httpbis-expect-ct-03#page-15
impl Default for ExpectCt {
    fn default() -> ExpectCt {
        ExpectCt::Enforce(Duration::days(30))
    }
}

impl From<&ExpectCt> for Header<'static> {
    fn from(expect: &ExpectCt) -> Self {
        let policy_string =  match expect {
            ExpectCt::Enforce(age) => format!("max-age={}, enforce", age.whole_seconds()),
            ExpectCt::Report(age, uri) => {
                format!(r#"max-age={}, report-uri="{}""#, age.whole_seconds(), uri)
            }
            ExpectCt::ReportAndEnforce(age, uri) => {
                format!("max-age={}, enforce, report-uri=\"{}\"", age.whole_seconds(), uri)
            }
        };

        Header::new(ExpectCt::NAME, policy_string)
    }
}

/// The [X-Content-Type-Options] header: turns off [mime sniffing] which can
/// prevent certain [attacks].
///
/// [mime sniffing]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Basics_of_HTTP/MIME_types#MIME_sniffing
/// [X-Content-Type-Options]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/X-Content-Type-Options
/// [attacks]: https://blog.mozilla.org/security/2016/08/26/mitigating-mime-confusion-attacks-in-firefox/
pub enum NoSniff {
    /// Turns off mime sniffing.
    Enable,
}

/// Defaults to [`NoSniff::Enable`], turns off mime sniffing.
impl Default for NoSniff {
    fn default() -> NoSniff {
        NoSniff::Enable
    }
}

impl From<&NoSniff> for Header<'static> {
    fn from(_: &NoSniff) -> Self {
        Header::new(NoSniff::NAME, "nosniff")
    }
}

/// The HTTP [Strict-Transport-Security] (HSTS) header: enforces strict HTTPS
/// usage.
///
/// HSTS tells the browser that the site should only be accessed using HTTPS
/// instead of HTTP. HSTS prevents a variety of downgrading attacks and should
/// always be used when [TLS] is enabled. `Shield` will turn HSTS on and issue a
/// warning if you enable TLS without enabling HSTS when the application is run
/// in non-debug profiles.
///
/// While HSTS is important for HTTPS security, incorrectly configured HSTS can
/// lead to problems as you are disallowing access to non-HTTPS enabled parts of
/// your site. [Yelp engineering] has good discussion of potential challenges
/// that can arise and how to roll this out in a large scale setting. So, if
/// you use TLS, use HSTS, but roll it out with care.
///
/// [TLS]: https://rocket.rs/guide/configuration/#configuring-tls
/// [Strict-Transport-Security]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Strict-Transport-Security
/// [Yelp engineering]: https://engineeringblog.yelp.com/2017/09/the-road-to-hsts.html
#[derive(PartialEq, Copy, Clone)]
pub enum Hsts {
    /// Browser should only permit this site to be accesses by HTTPS for the
    /// next [`Duration`].
    Enable(Duration),

    /// Like [`Hsts::Enable`], but also apply to all of the site's subdomains.
    IncludeSubDomains(Duration),

    /// Google maintains an [HSTS preload service] that can be used to prevent
    /// the browser from ever connecting to your site over an insecure
    /// connection. Read more at [MDN]. Don't enable this before you have
    /// registered your site.
    ///
    /// [HSTS preload service]: https://hstspreload.org/
    /// [MDN]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Strict-Transport-Security#Preloading_Strict_Transport_Security
    Preload(Duration),
}

/// Defaults to `Hsts::Enable(Duration::days(365))`.
impl Default for Hsts {
    fn default() -> Hsts {
        Hsts::Enable(Duration::days(365))
    }
}

impl From<&Hsts> for Header<'static> {
    fn from(hsts: &Hsts) -> Self {
        if hsts == &Hsts::default() {
            static DEFAULT: Header<'static> = Header {
                name: Uncased::from_borrowed(Hsts::NAME),
                value: Cow::Borrowed("max-age=31536000")
            };

            return DEFAULT.clone();
        }

        let policy_string = match hsts {
            Hsts::Enable(age) => format!("max-age={}", age.whole_seconds()),
            Hsts::IncludeSubDomains(age) => {
                format!("max-age={}; includeSubDomains", age.whole_seconds())
            }
            Hsts::Preload(age) => format!("max-age={}; preload", age.whole_seconds()),
        };

        Header::new(Hsts::NAME, policy_string)
    }
}

/// The [X-Frame-Options] header: helps prevent [clickjacking] attacks.
///
/// Controls whether the browser should allow the page to render in a `<frame>`,
/// [`<iframe>`][iframe] or `<object>`. This can be used to prevent
/// [clickjacking] attacks.
///
/// [X-Frame-Options]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/X-Frame-Options
/// [clickjacking]: https://en.wikipedia.org/wiki/Clickjacking
/// [owasp-clickjacking]: https://www.owasp.org/index.php/Clickjacking_Defense_Cheat_Sheet
/// [iframe]: https://developer.mozilla.org/en-US/docs/Web/HTML/Element/iframe
pub enum Frame {
    /// Page cannot be displayed in a frame.
    Deny,

    /// Page can only be displayed in a frame if the page trying to render it is
    /// in the same origin. Interpretation of same-origin is [browser
    /// dependent][X-Frame-Options].
    ///
    /// [X-Frame-Options]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/X-Frame-Options
    SameOrigin,
}

/// Defaults to [`Frame::SameOrigin`].
impl Default for Frame {
    fn default() -> Frame {
        Frame::SameOrigin
    }
}

impl From<&Frame> for Header<'static> {
    fn from(frame: &Frame) -> Self {
        let policy_string: &'static str = match frame {
            Frame::Deny => "DENY",
            Frame::SameOrigin => "SAMEORIGIN",
        };

        Header::new(Frame::NAME, policy_string)
    }
}

/// The [X-XSS-Protection] header: filters some forms of reflected [XSS]
/// attacks. Modern browsers do not support or enforce this header.
///
/// [X-XSS-Protection]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/X-XSS-Protection
/// [XSS]: https://developer.mozilla.org/en-US/docs/Glossary/Cross-site_scripting
pub enum XssFilter {
    /// Disables XSS filtering.
    Disable,

    /// Enables XSS filtering. If XSS is detected, the browser will sanitize
    /// before rendering the page (_Shield default_).
    Enable,

    /// Enables XSS filtering. If XSS is detected, the browser will not
    /// render the page.
    EnableBlock,
}

/// Defaults to [`XssFilter::Enable`].
impl Default for XssFilter {
    fn default() -> XssFilter {
        XssFilter::Enable
    }
}

impl From<&XssFilter> for Header<'static> {
    fn from(filter: &XssFilter) -> Self {
        let policy_string: &'static str = match filter {
            XssFilter::Disable => "0",
            XssFilter::Enable => "1",
            XssFilter::EnableBlock => "1; mode=block",
        };

        Header::new(XssFilter::NAME, policy_string)
    }
}

/// The [X-DNS-Prefetch-Control] header: controls browser DNS prefetching.
///
/// Tells the browser if it should perform domain name resolution on both links
/// that the user may choose to follow as well as URLs for items referenced by
/// the document including images, CSS, JavaScript, and so forth. Disabling
/// prefetching is useful if you don't control the link on the pages, or know
/// that you don't want to leak information to these domains.
///
/// [X-DNS-Prefetch-Control]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/X-DNS-Prefetch-Control
pub enum Prefetch {
    /// Enables DNS prefetching. This is the browser default.
    On,
    /// Disables DNS prefetching. This is the shield policy default.
    Off,
}

impl Default for Prefetch {
    fn default() -> Prefetch {
        Prefetch::Off
    }
}

impl From<&Prefetch> for Header<'static> {
    fn from(prefetch: &Prefetch) -> Self {
        let policy_string = match prefetch {
            Prefetch::On => "on",
            Prefetch::Off => "off",
        };

        Header::new(Prefetch::NAME, policy_string)
    }
}

/// The [Permissions-Policy] header: allow or block the use of browser features.
///
/// Tells the browser to allow or block the use of a browser feature in the
/// top-level page as well as allow or block _requesting access to_ (via the
/// `allow` `iframe` attribute) features in embedded iframes.
///
/// By default, the top-level page may access ~all features and any embedded
/// iframes may request access to ~any feature. This header allows the server to
/// control exactly _which_ (if any) origins may access or request access to
/// browser features.
///
/// Features are enabled via the [`Permission::allowed()`] contructor and
/// chainable [`allow()`](Self::allow()) build method. Features can be blocked
/// via the [`Permission::blocked()`] and chainable [`block()`](Self::block())
/// builder method.
///
/// ```rust
    /// # #[macro_use] extern crate rocket;
/// use rocket::shield::{Shield, Permission, Feature, Allow};
///
/// // In addition to defaults, block access to geolocation and USB features.
/// // Enable camera and microphone features only for the serving origin. Enable
/// // payment request access for the current origin and `https://rocket.rs`.
/// let permission = Permission::default()
///     .block(Feature::Geolocation)
///     .block(Feature::Usb)
///     .allow(Feature::Camera, Allow::This)
///     .allow(Feature::Microphone, Allow::This)
///     .allow(Feature::Payment, [Allow::This, Allow::Origin(uri!("https://rocket.rs"))]);
///
/// rocket::build().attach(Shield::default().enable(permission));
/// ```
///
/// # Default
///
/// The default returned via [`Permission::default()`] blocks access to the
/// `interest-cohort` feature, otherwise known as FLoC, which disables using the
/// current site in ad targeting tracking computations.
///
/// [Permissions-Policy]: https://github.com/w3c/webappsec-permissions-policy/blob/a45df7b237e2a85e1909d7f226ca4eb4ce5095ba/permissions-policy-explainer.md
#[derive(PartialEq, Clone)]
pub struct Permission(IndexMap<Feature, Option<SmallVec<[Allow; 1]>>>);

impl Default for Permission {
    /// The default `Permission` policy blocks access to the `interest-cohort`
    /// feature, otherwise known as FLoC, which disables using the current site
    /// in ad targeting tracking computations.
    fn default() -> Self {
        Permission::blocked(Feature::InterestCohort)
    }
}

impl Permission {
    /// Constructs a new `Permission` policy with only `feature` allowed for the
    /// set of origins in `allow` which may be a single [`Allow`], a slice
    /// (`[Allow]` or `&[Allow]`), or a vector (`Vec<Allow>`).
    ///
    /// If `allow` is empty, the use of the feature is blocked unless another
    /// call to `allow()` allows it. If `allow` contains [`Allow::Any`], the
    /// feature is allowable for all origins. Otherwise, the feature is
    /// allowable only for the origin specified in `allow`.
    ///
    /// # Panics
    ///
    /// Panics if an `Absolute` URI in an `Allow::Origin` does not contain a
    /// host part.
    ///
    /// # Example
    ///
    /// ```rust
    /// # #[macro_use] extern crate rocket;
    /// use rocket::shield::{Permission, Feature, Allow};
    ///
    /// let rocket = Allow::Origin(uri!("https://rocket.rs"));
    ///
    /// let perm = Permission::allowed(Feature::Usb, Allow::This);
    /// let perm = Permission::allowed(Feature::Usb, Allow::Any);
    /// let perm = Permission::allowed(Feature::Usb, [Allow::This, rocket]);
    /// ```
    pub fn allowed<L>(feature: Feature, allow: L) -> Self
        where L: IntoCollection<Allow>
    {
        Permission(IndexMap::new()).allow(feature, allow)
    }

    /// Constructs a new `Permission` policy with only `feature` blocked.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::shield::{Permission, Feature};
    ///
    /// let perm = Permission::blocked(Feature::Usb);
    /// let perm = Permission::blocked(Feature::Payment);
    /// ```
    pub fn blocked(feature: Feature) -> Self {
        Permission(IndexMap::new()).block(feature)
    }

    /// Adds `feature` as allowable for the set of origins in `allow` which may
    /// be a single [`Allow`], a slice (`[Allow]` or `&[Allow]`), or a vector
    /// (`Vec<Allow>`).
    ///
    /// This policy supercedes any previous policy set for `feature`.
    ///
    /// If `allow` is empty, the use of the feature is blocked unless another
    /// call to `allow()` allows it. If `allow` contains [`Allow::Any`], the
    /// feature is allowable for all origins. Otherwise, the feature is
    /// allowable only for the origin specified in `allow`.
    ///
    /// # Panics
    ///
    /// Panics if an `Absolute` URI in an `Allow::Origin` does not contain a
    /// host part.
    ///
    /// # Example
    ///
    /// ```rust
    /// # #[macro_use] extern crate rocket;
    /// use rocket::shield::{Permission, Feature, Allow};
    ///
    /// let rocket = Allow::Origin(uri!("https://rocket.rs"));
    /// let perm = Permission::allowed(Feature::Usb, Allow::This)
    ///     .allow(Feature::Payment, [rocket, Allow::This]);
    /// ```
    pub fn allow<L>(mut self, feature: Feature, allow: L) -> Self
        where L: IntoCollection<Allow>
    {
        let mut allow = allow.into_collection();

        if allow.contains(&Allow::Any) {
            allow = Allow::Any.into_collection();
        }

        for allow in &allow {
            if let Allow::Origin(absolute) = allow {
                let auth = absolute.authority();
                if auth.is_none() || matches!(auth, Some(a) if a.host().is_empty()) {
                    panic!("...")
                }
            }
        }

        self.0.insert(feature, Some(allow));
        self
    }

    /// Blocks `feature`. This policy supercedes any previous policy set for
    /// `feature`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::shield::{Permission, Feature};
    ///
    /// let perm = Permission::default()
    ///     .block(Feature::Usb)
    ///     .block(Feature::Payment);
    /// ```
    pub fn block(mut self, feature: Feature) -> Self {
        self.0.insert(feature, None);
        self
    }

    /// Returns the allow list (so far) for `feature` if feature is allowed.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::shield::{Permission, Feature, Allow};
    ///
    /// let perm = Permission::default();
    /// assert!(perm.get(Feature::Usb).is_none());
    ///
    /// let perm = perm.allow(Feature::Usb, Allow::Any);
    /// assert_eq!(perm.get(Feature::Usb).unwrap(), &[Allow::Any]);
    /// ```
    pub fn get(&self, feature: Feature) -> Option<&[Allow]> {
        self.0.get(&feature)?.as_deref()
    }

    /// Returns an iterator over the pairs of features and their allow lists,
    /// `None` if the feature is blocked.
    ///
    /// Features are returned in the order in which they were first added.
    ///
    /// # Example
    ///
    /// ```rust
    /// # #[macro_use] extern crate rocket;
    /// use rocket::shield::{Permission, Feature, Allow};
    ///
    /// let foo = uri!("https://foo.com:1234");
    /// let perm = Permission::blocked(Feature::Camera)
    ///     .allow(Feature::Gyroscope, [Allow::This, Allow::Origin(foo.clone())])
    ///     .block(Feature::Payment)
    ///     .allow(Feature::Camera, Allow::Any);
    ///
    /// let perms: Vec<_> = perm.iter().collect();
    /// assert_eq!(perms.len(), 3);
    /// assert_eq!(perms, vec![
    ///     (Feature::Camera, Some(&[Allow::Any][..])),
    ///     (Feature::Gyroscope, Some(&[Allow::This, Allow::Origin(foo)][..])),
    ///     (Feature::Payment, None),
    /// ]);
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = (Feature, Option<&[Allow]>)> {
        self.0.iter().map(|(feature, list)| (*feature, list.as_deref()))
    }
}

impl From<&Permission> for Header<'static> {
    fn from(perm: &Permission) -> Self {
        if perm == &Permission::default() {
            static DEFAULT: Header<'static> = Header {
                name: Uncased::from_borrowed(Permission::NAME),
                value: Cow::Borrowed("interest-cohort=()")
            };

            return DEFAULT.clone();
        }

        let value = perm.0.iter()
            .map(|(feature, allow)| {
                let list = allow.as_ref()
                    .into_iter()
                    .flatten()
                    .map(|origin| origin.rendered())
                    .collect::<Vec<_>>()
                    .join(" ");

                format!("{}=({})", feature, list)
            })
            .collect::<Vec<_>>()
            .join(", ");

        Header::new(Permission::NAME, value)
    }
}

/// Specifies the origin(s) allowed to access a browser [`Feature`] via
/// [`Permission`].
#[derive(Debug, PartialEq, Clone)]
pub enum Allow {
    /// Allow this specific origin. The feature is allowed only for this
    /// specific origin.
    ///
    /// The `user_info`, `path`, and `query` parts of the URI, if any, are
    /// ignored.
    Origin(Absolute<'static>),
    /// Any origin at all.
    ///
    /// The feature will be allowed in all browsing contexts regardless of their
    /// origin.
    Any,
    /// The current origin.
    ///
    /// The feature will be allowed in the immediately returned document and in
    /// all nested browsing contexts (iframes) in the same origin.
    This,
}

impl Allow {
    fn rendered(&self) -> Cow<'static, str> {
        match self {
            Allow::Origin(uri) => {
                let mut string = String::with_capacity(32);
                string.push('"');
                string.push_str(uri.scheme());

                // This should never fail when rendering a header for `Shield`
                // due to `panic` in `.allow()`.
                if let Some(auth) = uri.authority() {
                    use std::fmt::Write;

                    let _ = write!(string, "://{}", auth.host());
                    if let Some(port) = auth.port() {
                        let _ = write!(string, ":{}", port);
                    }
                }

                string.push('"');
                string.into()
            }
            Allow::Any => "*".into(),
            Allow::This => "self".into(),
        }
    }
}

/// A browser feature that can be enabled or blocked via [`Permission`].
#[derive(Debug, PartialEq, Eq, Hash, Copy, Clone)]
#[non_exhaustive]
pub enum Feature {
    // Standardized.

    /// The "accelerometer" feature.
    Accelerometer,
    /// The "ambient-light-sensor" feature.
    AmbientLightSensor,
    /// The "autoplay" feature.
    Autoplay,
    /// The "battery" feature.
    Battery,
    /// The "camera" feature.
    Camera,
    /// The "cross-origin-isolated" feature.
    CrossOriginIsolated,
    /// The "display-capture" feature.
    Displaycapture,
    /// The "document-domain" feature.
    DocumentDomain,
    /// The "encrypted-media" feature.
    EncryptedMedia,
    /// The "execution-while-not-rendered" feature.
    ExecutionWhileNotRendered,
    /// The "execution-while-out-of-viewport" feature.
    ExecutionWhileOutOfviewport,
    /// The "fullscreen" feature.
    Fullscreen,
    /// The "geolocation" feature.
    Geolocation,
    /// The "gyroscope" feature.
    Gyroscope,
    /// The "magnetometer" feature.
    Magnetometer,
    /// The "microphone" feature.
    Microphone,
    /// The "midi" feature.
    Midi,
    /// The "navigation-override" feature.
    NavigationOverride,
    /// The "payment" feature.
    Payment,
    /// The "picture-in-picture" feature.
    PictureInPicture,
    /// The "publickey-credentials-get" feature.
    PublickeyCredentialsGet,
    /// The "screen-wake-lock" feature.
    ScreenWakeLock,
    /// The "sync-xhr" feature.
    SyncXhr,
    /// The "usb" feature.
    Usb,
    /// The "web-share" feature.
    WebShare,
    /// The "xr-spatial-tracking" feature.
    XrSpatialTracking,

    // Proposed.

    /// The "clipboard-read" feature.
    ClipboardRead,
    /// The "clipboard-write" feature.
    ClipboardWrite,
    /// The "gamepad" feature.
    Gamepad,
    /// The "speaker-selection" feature.
    SpeakerSelection,
    /// The "interest-cohort" feature.
    InterestCohort,

    // Experimental.

    /// The "conversion-measurement" feature.
    ConversionMeasurement,
    /// The "focus-without-user-activation" feature.
    FocusWithoutUserActivation,
    /// The "hid" feature.
    Hid,
    /// The "idle-detection" feature.
    IdleDetection,
    /// The "serial" feature.
    Serial,
    /// The "sync-script" feature.
    SyncScript,
    /// The "trust-token-redemption" feature.
    TrustTokenRedemption,
    /// The "vertical-scroll" feature.
    VerticalScroll,
}

impl Feature {
    /// Returns the feature string as it appears in the header.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::shield::Feature;
    ///
    /// assert_eq!(Feature::Camera.as_str(), "camera");
    /// assert_eq!(Feature::SyncScript.as_str(), "sync-script");
    /// ```
    pub const fn as_str(self) -> &'static str {
        use Feature::*;

        match self {
            Accelerometer => "accelerometer",
            AmbientLightSensor => "ambient-light-sensor",
            Autoplay => "autoplay",
            Battery => "battery",
            Camera => "camera",
            CrossOriginIsolated => "cross-origin-isolated",
            Displaycapture => "display-capture",
            DocumentDomain => "document-domain",
            EncryptedMedia => "encrypted-media",
            ExecutionWhileNotRendered => "execution-while-not-rendered",
            ExecutionWhileOutOfviewport => "execution-while-out-of-viewport",
            Fullscreen => "fullscreen",
            Geolocation => "geolocation",
            Gyroscope => "gyroscope",
            Magnetometer => "magnetometer",
            Microphone => "microphone",
            Midi => "midi",
            NavigationOverride => "navigation-override",
            Payment => "payment",
            PictureInPicture => "picture-in-picture",
            PublickeyCredentialsGet => "publickey-credentials-get",
            ScreenWakeLock => "screen-wake-lock",
            SyncXhr => "sync-xhr",
            Usb => "usb",
            WebShare => "web-share",
            XrSpatialTracking => "xr-spatial-tracking",

            ClipboardRead => "clipboard-read",
            ClipboardWrite => "clipboard-write",
            Gamepad => "gamepad",
            SpeakerSelection => "speaker-selection",
            InterestCohort => "interest-cohort",

            ConversionMeasurement => "conversion-measurement",
            FocusWithoutUserActivation => "focus-without-user-activation",
            Hid => "hid",
            IdleDetection => "idle-detection",
            Serial => "serial",
            SyncScript => "sync-script",
            TrustTokenRedemption => "trust-token-redemption",
            VerticalScroll => "vertical-scroll",
        }
    }
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}
