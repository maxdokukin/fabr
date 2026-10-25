//! Cartesia helpers for a typed or spoken request title. No speech UI is mounted
//! and nothing here calls the Cartesia API. A missing key leaves speech unwired.

use fabr_core::FabrError;

pub fn request_title(input: &str) -> Result<String, FabrError> {
    let title = input.trim();
    if title.is_empty() {
        return Err(FabrError::BadRequest("request is empty".into()));
    }
    Ok(title.to_string())
}

pub fn spoken_feedback(notifier_text: &str) -> &str {
    notifier_text
}

pub fn available(key_present: bool) -> bool {
    key_present
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spoken_and_typed_requests_match() {
        let spoken = request_title("  I'm starting my internship  ").unwrap();
        let typed = request_title("I'm starting my internship").unwrap();
        assert_eq!(spoken, typed);
        assert!(request_title("   ").is_err());
        assert_eq!(spoken_feedback("result succeeded"), "result succeeded");
        assert!(!available(false));
    }
}
