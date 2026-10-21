//! Startup check for the CPU instruction set extensions the node needs.
//!
//! On `x86_64` the prover's field arithmetic is compiled to use ADX (`adcx`,
//! `adox`) and BMI2 (`mulx`) instructions without a runtime fallback, so on a
//! CPU lacking them the node would later crash with an illegal instruction.

#[derive(Debug, thiserror::Error)]
#[error(
    "this CPU does not support the required instruction set extension(s): {}. The node needs \
     an Intel Broadwell (5th generation) or newer CPU, or any AMD Zen CPU.",
    missing.join(", ")
)]
pub struct MissingCpuFeaturesError {
    missing: Vec<&'static str>,
}

/// Fails with a clear error if the CPU lacks an instruction set extension the
/// node requires, instead of letting the node crash later.
///
/// # Errors
///
/// Returns [`MissingCpuFeaturesError`] naming every missing extension.
#[cfg(target_arch = "x86_64")]
pub fn ensure_required_cpu_features() -> Result<(), MissingCpuFeaturesError> {
    let detected = [
        ("adx", std::arch::is_x86_feature_detected!("adx")),
        ("bmi2", std::arch::is_x86_feature_detected!("bmi2")),
    ];
    let missing = missing_features(&detected);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(MissingCpuFeaturesError { missing })
    }
}

/// No extension is required beyond the target's baseline on other
/// architectures.
///
/// # Errors
///
/// Never fails.
#[cfg(not(target_arch = "x86_64"))]
pub const fn ensure_required_cpu_features() -> Result<(), MissingCpuFeaturesError> {
    Ok(())
}

#[cfg(any(target_arch = "x86_64", test))]
fn missing_features(detected: &[(&'static str, bool)]) -> Vec<&'static str> {
    detected
        .iter()
        .filter(|(_, is_detected)| !is_detected)
        .map(|(feature, _)| *feature)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_features_lists_only_undetected_features() {
        let detected = [("adx", false), ("bmi2", true)];

        assert_eq!(missing_features(&detected), vec!["adx"]);
    }

    #[test]
    fn missing_features_is_empty_when_all_are_detected() {
        let detected = [("adx", true), ("bmi2", true)];

        assert_eq!(missing_features(&detected), Vec::<&str>::new());
    }

    #[test]
    fn error_names_the_missing_features_and_supported_cpus() {
        let message = MissingCpuFeaturesError {
            missing: vec!["adx"],
        }
        .to_string();

        assert!(message.contains("adx"), "{message}");
        assert!(message.contains("Broadwell"), "{message}");
        assert!(message.contains("Zen"), "{message}");
    }
}
