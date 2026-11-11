//! What the updater is doing, as the widget and the settings window render it, and how each step
//! of a check changes it. No I/O here, so the tests run on any platform.
//!
//! Only a check the user asked for shows its progress and result. A background check shows
//! nothing until it has a verified installer: its "up to date" and its failures leave `Idle`.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Phase {
    /// Nothing to show: no check yet, or the last one ran in the background and found nothing.
    Idle,
    /// A check the user asked for is running.
    Checking,
    /// A check the user asked for is downloading this version's installer.
    Downloading { version: String },
    /// The user's check found nothing newer than the running app.
    UpToDate,
    /// The user's check, or the start of the installer, failed.
    Failed { message: String },
    /// A verified installer for `version` is on disk. `dismissed`: the user chose "Later", so the
    /// widget hides its banner until a newer version turns up or the app restarts.
    Ready {
        version: String,
        dismissed: bool,
        #[serde(skip)]
        installer: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    #[serde(flatten)]
    pub phase: Phase,
    /// When the last check finished, whatever it found (ms since the Unix epoch).
    pub last_checked: Option<u64>,
    pub current_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Background,
    Manual,
}

/// How a check ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ready { version: String, installer: PathBuf },
    UpToDate,
    Failed(String),
}

/// The status plus the check in progress, if any. One check runs at a time, so a tray click
/// during a background check neither downloads twice nor starts a second request.
#[derive(Debug)]
pub struct Machine {
    status: UpdateStatus,
    running: Option<Trigger>,
}

impl Machine {
    pub fn new(current_version: String) -> Self {
        Self {
            status: UpdateStatus {
                phase: Phase::Idle,
                last_checked: None,
                current_version,
            },
            running: None,
        }
    }

    pub fn status(&self) -> &UpdateStatus {
        &self.status
    }

    /// A check is asked for. Returns true when the caller should run it; false when one is
    /// already running, in which case a manual request makes that check report as manual.
    pub fn begin(&mut self, trigger: Trigger) -> bool {
        if trigger == Trigger::Manual {
            match &mut self.status.phase {
                // The update is already here: show it again rather than a spinner; the check
                // still runs, quietly, in case a newer release came out since.
                Phase::Ready { dismissed, .. } => *dismissed = false,
                phase => *phase = Phase::Checking,
            }
        }
        match self.running {
            Some(_) => {
                if trigger == Trigger::Manual {
                    self.running = Some(Trigger::Manual);
                }
                false
            }
            None => {
                self.running = Some(trigger);
                true
            }
        }
    }

    /// The running check starts downloading `version` (no cached copy verified).
    pub fn downloading(&mut self, version: String) {
        if self.running == Some(Trigger::Manual) && self.status.phase == Phase::Checking {
            self.status.phase = Phase::Downloading { version };
        }
    }

    /// The running check ended at `now_ms`.
    pub fn finish(&mut self, outcome: Outcome, now_ms: u64) {
        let manual = self.running.take() == Some(Trigger::Manual);
        self.status.last_checked = Some(now_ms);
        let phase = &mut self.status.phase;
        *phase = match outcome {
            Outcome::Ready { version, installer } => {
                // "Later" holds for the version it was said to, until the user checks again.
                let dismissed = !manual
                    && matches!(phase, Phase::Ready { version: v, dismissed: true, .. } if *v == version);
                Phase::Ready {
                    version,
                    dismissed,
                    installer,
                }
            }
            Outcome::UpToDate if manual => Phase::UpToDate,
            // The installer already on disk stays good when GitHub cannot be reached.
            Outcome::Failed(_) if matches!(phase, Phase::Ready { .. }) => return,
            Outcome::Failed(message) if manual => Phase::Failed { message },
            Outcome::UpToDate | Outcome::Failed(_) => Phase::Idle,
        };
    }

    /// "Later": hide the banner for this version. Only meaningful in `Ready`.
    pub fn dismiss(&mut self) {
        if let Phase::Ready { dismissed, .. } = &mut self.status.phase {
            *dismissed = true;
        }
    }

    /// The verified installer to run, when an update is ready.
    pub fn installer(&self) -> Option<&Path> {
        match &self.status.phase {
            Phase::Ready { installer, .. } => Some(installer),
            _ => None,
        }
    }

    /// The installer could not be started.
    pub fn install_failed(&mut self, message: String) {
        self.status.phase = Phase::Failed { message };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn machine() -> Machine {
        Machine::new("1.4.0".into())
    }

    fn ready(version: &str) -> Outcome {
        Outcome::Ready {
            version: version.into(),
            installer: PathBuf::from(format!("C:/Temp/UsageDeck-{version}-setup.exe")),
        }
    }

    fn check(m: &mut Machine, trigger: Trigger, outcome: Outcome) {
        assert!(m.begin(trigger), "no check was running");
        m.finish(outcome, 1_000);
    }

    #[test]
    fn serializes_for_the_frontend_without_the_installer_path() {
        let mut m = machine();
        assert_eq!(
            serde_json::to_value(m.status()).unwrap(),
            json!({ "state": "idle", "lastChecked": null, "currentVersion": "1.4.0" })
        );
        check(&mut m, Trigger::Background, ready("1.5.0"));
        assert_eq!(
            serde_json::to_value(m.status()).unwrap(),
            json!({
                "state": "ready",
                "version": "1.5.0",
                "dismissed": false,
                "lastChecked": 1000,
                "currentVersion": "1.4.0"
            })
        );
        m.install_failed("access denied".into());
        assert_eq!(
            serde_json::to_value(m.status()).unwrap()["state"],
            json!("failed")
        );
        let mut m = machine();
        m.begin(Trigger::Manual);
        m.downloading("1.5.0".into());
        assert_eq!(
            serde_json::to_value(m.status()).unwrap(),
            json!({
                "state": "downloading",
                "version": "1.5.0",
                "lastChecked": null,
                "currentVersion": "1.4.0"
            })
        );
        m.finish(Outcome::UpToDate, 5);
        assert_eq!(
            serde_json::to_value(m.status()).unwrap()["state"],
            json!("upToDate")
        );
    }

    #[test]
    fn background_checks_stay_silent_unless_an_update_is_ready() {
        let mut m = machine();
        assert!(m.begin(Trigger::Background));
        assert_eq!(
            m.status().phase,
            Phase::Idle,
            "no spinner for a background check"
        );
        m.downloading("1.5.0".into());
        assert_eq!(m.status().phase, Phase::Idle, "no download progress either");
        m.finish(Outcome::UpToDate, 10);
        assert_eq!(m.status().phase, Phase::Idle);
        assert_eq!(m.status().last_checked, Some(10));

        check(
            &mut m,
            Trigger::Background,
            Outcome::Failed("offline".into()),
        );
        assert_eq!(m.status().phase, Phase::Idle);
        assert_eq!(m.status().last_checked, Some(1_000));
    }

    #[test]
    fn manual_checks_show_progress_and_every_result() {
        let mut m = machine();
        assert!(m.begin(Trigger::Manual));
        assert_eq!(m.status().phase, Phase::Checking);
        m.downloading("1.5.0".into());
        assert_eq!(
            m.status().phase,
            Phase::Downloading {
                version: "1.5.0".into()
            }
        );
        m.finish(ready("1.5.0"), 1);
        assert!(
            matches!(&m.status().phase, Phase::Ready { version, dismissed: false, .. } if version == "1.5.0")
        );

        let mut m = machine();
        check(&mut m, Trigger::Manual, Outcome::UpToDate);
        assert_eq!(m.status().phase, Phase::UpToDate);
        check(&mut m, Trigger::Manual, Outcome::Failed("offline".into()));
        assert_eq!(
            m.status().phase,
            Phase::Failed {
                message: "offline".into()
            }
        );
        // the next background check clears a result the user has seen
        check(&mut m, Trigger::Background, Outcome::UpToDate);
        assert_eq!(m.status().phase, Phase::Idle);
    }

    #[test]
    fn one_check_at_a_time_and_a_click_during_a_background_check_makes_it_manual() {
        let mut m = machine();
        assert!(m.begin(Trigger::Background));
        assert!(!m.begin(Trigger::Background));
        assert!(!m.begin(Trigger::Manual), "the running check is reused");
        assert_eq!(
            m.status().phase,
            Phase::Checking,
            "the click shows feedback"
        );
        m.finish(Outcome::UpToDate, 2);
        assert_eq!(
            m.status().phase,
            Phase::UpToDate,
            "reported as the user's check"
        );
        assert!(m.begin(Trigger::Background), "free again once finished");
    }

    #[test]
    fn later_hides_this_version_until_a_newer_one_or_a_manual_check() {
        let mut m = machine();
        check(&mut m, Trigger::Background, ready("1.5.0"));
        m.dismiss();
        assert!(matches!(
            m.status().phase,
            Phase::Ready {
                dismissed: true,
                ..
            }
        ));

        check(&mut m, Trigger::Background, ready("1.5.0"));
        assert!(
            matches!(
                m.status().phase,
                Phase::Ready {
                    dismissed: true,
                    ..
                }
            ),
            "the hourly check does not bring the same version back"
        );

        check(&mut m, Trigger::Background, ready("1.5.1"));
        assert!(
            matches!(&m.status().phase, Phase::Ready { version, dismissed: false, .. } if version == "1.5.1")
        );

        m.dismiss();
        assert!(m.begin(Trigger::Manual));
        assert!(
            matches!(
                m.status().phase,
                Phase::Ready {
                    dismissed: false,
                    ..
                }
            ),
            "asking again shows the ready update at once, not a spinner"
        );
        m.finish(ready("1.5.1"), 3);
        assert!(matches!(
            m.status().phase,
            Phase::Ready {
                dismissed: false,
                ..
            }
        ));
    }

    #[test]
    fn a_failed_check_keeps_an_installer_that_is_already_verified() {
        for trigger in [Trigger::Background, Trigger::Manual] {
            let mut m = machine();
            check(&mut m, Trigger::Background, ready("1.5.0"));
            check(&mut m, trigger, Outcome::Failed("offline".into()));
            assert_eq!(
                m.installer(),
                Some(Path::new("C:/Temp/UsageDeck-1.5.0-setup.exe")),
                "{trigger:?}"
            );
        }
    }

    #[test]
    fn install_is_only_possible_when_ready() {
        let mut m = machine();
        assert_eq!(m.installer(), None);
        check(&mut m, Trigger::Manual, Outcome::UpToDate);
        assert_eq!(m.installer(), None);
        m.dismiss();
        assert_eq!(
            m.status().phase,
            Phase::UpToDate,
            "Later outside Ready does nothing"
        );
        check(&mut m, Trigger::Background, ready("1.5.0"));
        m.dismiss();
        assert!(m.installer().is_some(), "Later keeps the download");
        m.install_failed("access denied".into());
        assert_eq!(m.installer(), None);
    }
}
