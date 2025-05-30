use nomos_sdp_core::{BlockNumber, StakeThreshold};
use serde::{Deserialize, Serialize};

pub mod declare;

/// Serialization proxy type for [`nomos_sdp_core::ServiceType`]
#[derive(Serialize, Deserialize)]
#[serde(remote = "nomos_sdp_core::ServiceType")]
enum ServiceType {
    #[serde(rename = "BN")]
    BlendNetwork,
    #[serde(rename = "DA")]
    DataAvailability,
    #[serde(rename = "EX")]
    ExecutorNetwork,
}

#[derive(Serialize, Deserialize)]
#[serde(transparent, remote = "nomos_sdp_core::Locator")]
pub struct Locator(multiaddr::Multiaddr);

#[derive(Serialize, Deserialize)]
#[serde(remote = "nomos_sdp_core::MinStake")]
pub struct MinStake {
    pub threshold: StakeThreshold,
    pub timestamp: BlockNumber,
}

mod tests {
    use nomos_sdp_core::ServiceType;
    use serde::{Deserialize, Serialize};
    use serde_json;
    #[derive(Serialize, Deserialize)]
    #[serde(transparent)]
    struct ServiceTypeTest(#[serde(with = "super::ServiceType")] nomos_sdp_core::ServiceType);
    #[test]
    fn test_service_type_serialization() {
        // Test serialization
        let blend_network = ServiceTypeTest(ServiceType::BlendNetwork);
        let serialized = serde_json::to_string(&blend_network).expect("Serialization failed");
        assert_eq!(serialized, "\"BN\"");

        let data_availability = ServiceTypeTest(ServiceType::DataAvailability);
        let serialized = serde_json::to_string(&data_availability).expect("Serialization failed");
        assert_eq!(serialized, "\"DA\"");

        let executor_network = ServiceTypeTest(ServiceType::ExecutorNetwork);
        let serialized = serde_json::to_string(&executor_network).expect("Serialization failed");
        assert_eq!(serialized, "\"EX\"");
    }

    #[test]
    fn test_service_type_deserialization() {
        // Test deserialization
        let serialized_blend_network = "\"BN\"";
        let deserialized: ServiceTypeTest =
            serde_json::from_str(serialized_blend_network).expect("Deserialization failed");
        assert!(matches!(
            deserialized,
            ServiceTypeTest(ServiceType::BlendNetwork)
        ));

        let serialized_data_availability = "\"DA\"";
        let deserialized: ServiceTypeTest =
            serde_json::from_str(serialized_data_availability).expect("Deserialization failed");
        assert!(matches!(
            deserialized,
            ServiceTypeTest(ServiceType::DataAvailability)
        ));

        let serialized_executor_network = "\"EX\"";
        let deserialized: ServiceTypeTest =
            serde_json::from_str(serialized_executor_network).expect("Deserialization failed");
        assert!(matches!(
            deserialized,
            ServiceTypeTest(ServiceType::ExecutorNetwork)
        ));
    }
}
