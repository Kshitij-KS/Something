//! Notification delivery behind a testable sink.
#![cfg_attr(windows, allow(unsafe_code))]

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

pub const CALLBACK_APP_ID: &str = "com.callback.desktop";
pub const ACTIONABLE_REMINDER_TITLE: &str = "Callback";
pub const ACTIONABLE_REMINDER_BODY: &str = "A reminder is ready in Callback.";

/// Payload shown on a surface attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationRequest {
    pub title: String,
    pub body: String,
    pub action_token: Option<String>,
}

impl NotificationRequest {
    /// Builds an actionable request with fixed copy so captured text never
    /// enters Windows notification or lock-screen history.
    #[must_use]
    pub fn actionable(token: &str) -> Self {
        Self {
            title: ACTIONABLE_REMINDER_TITLE.into(),
            body: ACTIONABLE_REMINDER_BODY.into(),
            action_token: Some(token.to_owned()),
        }
    }

    #[must_use]
    pub fn informational(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            action_token: None,
        }
    }
}

/// Delivery or OS-history failure.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum NotifyError {
    #[error("notification delivery failed: {0}")]
    Delivery(String),
    #[error("notification history cleanup failed: {0}")]
    HistoryCleanup(String),
}

/// OS-backed or test notification sink.
pub trait NotificationSink: Send + Sync {
    /// Shows a toast or records it in tests.
    ///
    /// # Errors
    ///
    /// Returns [`NotifyError`] when the platform adapter cannot deliver.
    fn show(&self, request: &NotificationRequest) -> Result<(), NotifyError>;
}

/// In-memory sink used by unit tests.
#[derive(Debug, Default)]
pub struct RecordingSink {
    shown: Mutex<Vec<NotificationRequest>>,
}

impl RecordingSink {
    #[must_use]
    pub fn shown(&self) -> Vec<NotificationRequest> {
        self.shown
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

impl NotificationSink for RecordingSink {
    fn show(&self, request: &NotificationRequest) -> Result<(), NotifyError> {
        self.shown
            .lock()
            .map_err(|_| NotifyError::Delivery("poisoned".into()))?
            .push(request.clone());
        Ok(())
    }
}

/// Builds a toast payload on every platform so CI can verify action wiring.
#[must_use]
pub fn toast_xml(request: &NotificationRequest) -> String {
    let title = xml_escape(&request.title);
    let body = xml_escape(&request.body);
    let (launch, actions) = request.action_token.as_ref().map_or_else(
        || (String::new(), String::new()),
        |token| {
            let token = xml_escape(token);
            (
                format!(
                    " activationType=\"protocol\" launch=\"callback-action://open/{token}\""
                ),
                format!(
                    "<actions>\
                       <action content=\"Done\" activationType=\"protocol\" arguments=\"callback-action://done/{token}\"/>\
                       <action content=\"Snooze\" activationType=\"protocol\" arguments=\"callback-action://snooze/{token}\"/>\
                       <action content=\"Not a promise\" activationType=\"protocol\" arguments=\"callback-action://reject/{token}\"/>\
                       <action content=\"Ignore\" activationType=\"protocol\" arguments=\"callback-action://ignore/{token}\"/>\
                     </actions>"
                ),
            )
        },
    );
    format!(
        "<toast{launch}><visual><binding template=\"ToastGeneric\">\
           <text>{title}</text><text>{body}</text>\
         </binding></visual>{actions}</toast>"
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Removes all Callback notifications from platform history.
///
/// # Errors
///
/// Returns [`NotifyError::HistoryCleanup`] when Windows Runtime initialization
/// or Action Center history cleanup fails.
pub fn clear_callback_history() -> Result<(), NotifyError> {
    #[cfg(all(target_os = "windows", feature = "windows-platform"))]
    {
        windows::clear_callback_history()
    }
    #[cfg(not(all(target_os = "windows", feature = "windows-platform")))]
    {
        Ok(())
    }
}

/// Selects the compiled platform sink.
#[must_use]
pub fn platform_sink() -> Box<dyn NotificationSink> {
    #[cfg(all(target_os = "windows", feature = "windows-platform"))]
    {
        Box::new(windows::WindowsToastSink)
    }
    #[cfg(not(all(target_os = "windows", feature = "windows-platform")))]
    {
        Box::new(RecordingSink::default())
    }
}

#[cfg(all(target_os = "windows", feature = "windows-platform"))]
mod windows {
    use super::{CALLBACK_APP_ID, NotificationRequest, NotificationSink, NotifyError, toast_xml};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
    use windows::core::HSTRING;

    pub struct WindowsToastSink;

    impl NotificationSink for WindowsToastSink {
        fn show(&self, request: &NotificationRequest) -> Result<(), NotifyError> {
            let document =
                XmlDocument::new().map_err(|error| NotifyError::Delivery(error.to_string()))?;
            document
                .LoadXml(&HSTRING::from(toast_xml(request)))
                .map_err(|error| NotifyError::Delivery(error.to_string()))?;
            let toast = ToastNotification::CreateToastNotification(&document)
                .map_err(|error| NotifyError::Delivery(error.to_string()))?;
            let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
                CALLBACK_APP_ID,
            ))
            .map_err(|error| NotifyError::Delivery(error.to_string()))?;
            notifier
                .Show(&toast)
                .map_err(|error| NotifyError::Delivery(error.to_string()))
        }
    }

    pub(super) fn clear_callback_history() -> Result<(), NotifyError> {
        let _runtime = WinRtInitialization::new()?;
        let history = ToastNotificationManager::History()
            .map_err(|error| NotifyError::HistoryCleanup(error.to_string()))?;
        history
            .ClearWithId(&HSTRING::from(CALLBACK_APP_ID))
            .map_err(|error| NotifyError::HistoryCleanup(error.to_string()))
    }

    struct WinRtInitialization;

    impl WinRtInitialization {
        fn new() -> Result<Self, NotifyError> {
            // SAFETY: this helper owns the matching RoUninitialize call on the
            // same thread whenever initialization succeeds.
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
                .map_err(|error| NotifyError::HistoryCleanup(error.to_string()))?;
            Ok(Self)
        }
    }

    impl Drop for WinRtInitialization {
        fn drop(&mut self) {
            // SAFETY: construction succeeded on this thread and each instance
            // balances exactly one successful RoInitialize call.
            unsafe { RoUninitialize() };
        }
    }
}
