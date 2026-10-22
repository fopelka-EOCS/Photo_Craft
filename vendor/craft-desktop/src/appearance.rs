//! Portal appearance, without UI dependencies or synchronous UI-thread D-Bus calls.
//! A bounded two-second poll also recovers from portal restarts. Dropping the handle
//! wakes the worker immediately; no permanently blocked signal listener is leaked.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Appearance {
    pub dark: Option<bool>,
    /// sRGB channels in 0..=1. Absent/invalid values leave the application's accent intact.
    pub accent: Option<[f64; 3]>,
}

#[derive(Default)]
pub struct Watcher {
    changes: Option<std::sync::mpsc::Receiver<Appearance>>,
    _stop: Option<std::sync::mpsc::Sender<()>>,
}

impl Watcher {
    pub fn start(repaint: impl Fn() + Send + 'static) -> Self {
        #[cfg(all(target_os = "linux", feature = "portal"))]
        {
            use std::sync::mpsc;
            let (tx, changes) = mpsc::channel();
            let (stop, cancelled) = mpsc::channel();
            let started = std::thread::Builder::new()
                .name("craft-appearance".into())
                .spawn(move || {
                    let mut bus = None;
                    let mut previous = None;
                    loop {
                        if let Some(current) = read(&mut bus)
                            && previous != Some(current)
                        {
                            previous = Some(current);
                            if tx.send(current).is_err() {
                                return;
                            }
                            repaint();
                        }
                        match cancelled.recv_timeout(std::time::Duration::from_secs(2)) {
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            _ => return,
                        }
                    }
                });
            if started.is_ok() {
                Self {
                    changes: Some(changes),
                    _stop: Some(stop),
                }
            } else {
                Self::default()
            }
        }
        #[cfg(not(all(target_os = "linux", feature = "portal")))]
        {
            let _ = repaint;
            Self::default()
        }
    }

    pub fn watching(&self) -> bool {
        self.changes.is_some()
    }

    pub fn take_change(&mut self) -> Option<Appearance> {
        self.changes.as_ref()?.try_iter().last()
    }
}

pub fn decode_scheme(value: u32) -> Option<bool> {
    match value {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

pub fn validate_accent(rgb: [f64; 3]) -> Option<[f64; 3]> {
    rgb.iter()
        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        .then_some(rgb)
}

#[cfg(all(target_os = "linux", feature = "portal"))]
fn unpack(mut value: zbus::zvariant::Value<'_>) -> zbus::zvariant::Value<'_> {
    for _ in 0..4 {
        match value {
            zbus::zvariant::Value::Value(inner) => value = *inner,
            other => return other,
        }
    }
    value
}

#[cfg(all(target_os = "linux", feature = "portal"))]
fn read(bus: &mut Option<zbus::blocking::Connection>) -> Option<Appearance> {
    use zbus::{blocking::connection::Builder, zvariant::OwnedValue};
    if bus.is_none() {
        *bus = Builder::session()
            .ok()?
            .method_timeout(std::time::Duration::from_secs(1))
            .build()
            .ok();
    }
    let connection = bus.as_ref()?;
    let query = |key| -> Option<OwnedValue> {
        // Read is supported by older Settings implementations too. It may wrap a variant twice.
        connection
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Settings"),
                "Read",
                &("org.freedesktop.appearance", key),
            )
            .ok()?
            .body()
            .deserialize()
            .ok()
    };
    let scheme = query("color-scheme");
    let accent = query("accent-color");
    if scheme.is_none() && accent.is_none() {
        *bus = None;
        return None; // Keep the last successful answer while the portal is restarting.
    }
    Some(Appearance {
        dark: scheme
            .and_then(|v| u32::try_from(unpack(v.into())).ok())
            .and_then(decode_scheme),
        accent: accent
            .and_then(|v| <(f64, f64, f64)>::try_from(unpack(v.into())).ok())
            .and_then(|(r, g, b)| validate_accent([r, g, b])),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(all(target_os = "linux", feature = "portal"))]
    struct Settings(std::sync::Arc<std::sync::Mutex<u32>>);

    #[cfg(all(target_os = "linux", feature = "portal"))]
    #[zbus::interface(name = "org.freedesktop.portal.Settings")]
    impl Settings {
        fn read(
            &self,
            namespace: &str,
            key: &str,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedValue> {
            if namespace != "org.freedesktop.appearance" {
                return Err(zbus::fdo::Error::InvalidArgs("namespace".into()));
            }
            match key {
                "color-scheme" => self
                    .0
                    .lock()
                    .map(|v| (*v).into())
                    .map_err(|e| zbus::fdo::Error::Failed(e.to_string())),
                "accent-color" => zbus::zvariant::OwnedValue::try_from(
                    zbus::zvariant::Value::from((0.2f64, 0.4f64, 0.6f64)),
                )
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string())),
                _ => Err(zbus::fdo::Error::InvalidArgs("key".into())),
            }
        }
    }

    #[test]
    #[ignore = "requires a private D-Bus session; never run against the user's portal"]
    #[cfg(all(target_os = "linux", feature = "portal"))]
    #[allow(clippy::unwrap_used)]
    fn appearance_dbus_live_changes_and_drop_stop_worker() {
        use std::{
            sync::{Arc, Mutex},
            time::{Duration, Instant},
        };
        let value = Arc::new(Mutex::new(1));
        let _server = zbus::blocking::connection::Builder::session()
            .unwrap()
            .name("org.freedesktop.portal.Desktop")
            .unwrap()
            .serve_at("/org/freedesktop/portal/desktop", Settings(value.clone()))
            .unwrap()
            .build()
            .unwrap();
        let marker = Arc::new(());
        let weak = Arc::downgrade(&marker);
        let mut watcher = Watcher::start(move || {
            let _ = &marker;
        });
        let rx = watcher.changes.as_mut().unwrap();
        let first = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(first.dark, Some(true));
        assert_eq!(first.accent, Some([0.2, 0.4, 0.6]));
        *value.lock().unwrap() = 2;
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(4)).unwrap().dark,
            Some(false)
        );
        *value.lock().unwrap() = 0;
        assert_eq!(rx.recv_timeout(Duration::from_secs(4)).unwrap().dark, None);
        drop(watcher);
        let deadline = Instant::now() + Duration::from_secs(3);
        while weak.upgrade().is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            weak.upgrade().is_none(),
            "watcher must stop even if appearance never changes again"
        );
    }
    #[test]
    fn standard_portal_codes_and_finite_accent() {
        assert_eq!(decode_scheme(0), None);
        assert_eq!(decode_scheme(1), Some(true));
        assert_eq!(decode_scheme(2), Some(false));
        assert_eq!(decode_scheme(99), None);
        assert_eq!(validate_accent([0., 0.5, 1.]), Some([0., 0.5, 1.]));
        assert_eq!(validate_accent([f64::NAN, 0., 0.]), None);
        assert_eq!(validate_accent([0., 1.1, 0.]), None);
    }
    #[test]
    fn default_does_not_access_desktop() {
        let mut watcher = Watcher::default();
        assert!(!watcher.watching());
        assert_eq!(watcher.take_change(), None);
    }
    #[test]
    #[cfg(all(target_os = "linux", feature = "portal"))]
    fn nested_variants_are_supported() {
        use zbus::zvariant::Value;
        assert_eq!(
            u32::try_from(unpack(Value::Value(Box::new(Value::U32(1))))).ok(),
            Some(1)
        );
    }
}
