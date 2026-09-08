//! Accessibility compliance checking for DCP packages.
//!
//! Delegates to [`postkit::accessibility`] for checking DCP content against
//! accessibility standards (CVAA, EAA, AODA, Ofcom).

pub use postkit::accessibility::{
    AccessibilityFinding, AccessibilityResult, AccessibilityStandard, AccessibilityTrack, Severity,
    check_accessibility, check_accessibility_multi,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_package_fails_cvaa() {
        let dir = tempfile::tempdir().unwrap();
        let result = check_accessibility(dir.path(), AccessibilityStandard::Cvaa);
        assert!(!result.compliant);
        assert_eq!(result.errors, 2); // CC + AD required
    }

    #[test]
    fn test_empty_package_fails_eaa() {
        let dir = tempfile::tempdir().unwrap();
        let result = check_accessibility(dir.path(), AccessibilityStandard::Eaa);
        assert!(!result.compliant);
        assert_eq!(result.errors, 2); // CC + AD required
    }

    #[test]
    fn test_multi_standard_check() {
        let dir = tempfile::tempdir().unwrap();
        let results = check_accessibility_multi(
            dir.path(),
            &[AccessibilityStandard::Cvaa, AccessibilityStandard::Ofcom],
        );
        assert_eq!(results.len(), 2);
        assert!(!results[0].compliant);
        assert!(!results[1].compliant);
    }
}
