//! Notification delivery behind a testable sink.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// Payload shown on a surface attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationRequest {
    pub title: String,
    pub body: String,
    pub action_token: String,
}

/// Delivery failure.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum NotifyError {
    #[error("notification delivery failed: {0}")]
    Delivery(String),
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
    use super::{NotificationRequest, NotificationSink, NotifyError};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::core::HSTRING;

    pub struct WindowsToastSink;

    impl NotificationSink for WindowsToastSink {
        fn show(&self, request: &NotificationRequest) -> Result<(), NotifyError> {
            let xml = toast_xml(&request.title, &request.body, &request.action_token);
            let document =
                XmlDocument::new().map_err(|error| NotifyError::Delivery(error.to_string()))?;
            document
                .LoadXml(&HSTRING::from(xml))
                .map_err(|error| NotifyError::Delivery(error.to_string()))?;
            let toast = ToastNotification::CreateToastNotification(&document)
                .map_err(|error| NotifyError::Delivery(error.to_string()))?;
            let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
                "com.callback.desktop",
            ))
            .map_err(|error| NotifyError::Delivery(error.to_string()))?;
            notifier
                .Show(&toast)
                .map_err(|error| NotifyError::Delivery(error.to_string()))
        }
    }

    fn toast_xml(title: &str, body: &str, token: &str) -> String {
        format!(
            "<toast launch=\"{token}\">\
               <visual><binding template=\"ToastGeneric\">\
                 <text>{}</text><text>{}</text>\
               </binding></visual>\
               <actions>\
                 <action content=\"Done\" arguments=\"done:{token}\"/>\
                 <action content=\"Snooze\" arguments=\"snooze:{token}\"/>\
                 <action content=\"Not a promise\" arguments=\"reject:{token}\"/>\
               </actions>\
             </toast>",
            xml_escape(title),
            xml_escape(body)
        )
    }

    fn xml_escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
}
