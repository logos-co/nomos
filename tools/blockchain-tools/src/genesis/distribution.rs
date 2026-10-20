// TODO(RFC-480): the distribution is a list of genesis allocations.
// use std::collections::HashSet;
use std::collections::{HashMap, HashSet};

use lb_core::{
    mantle::{
        // TODO(RFC-480): the distribution is a list of genesis allocations.
        // Note, Utxo, Value as NoteValue,
        // ledger::{Inputs, Outputs},
        // ops::{sdp::SDPDeclareOp, transfer::TransferOp},
        Value as NoteValue,
        note_set::CommitmentMmr,
        notes::derive_note_nf,
        ops::sdp::SDPDeclareOp,
        transactions::GenesisAllocation,
    },
    sdp::ServiceType,
};
// TODO(RFC-480): a declaration spends the stakeholder note with its stake key.
// use lb_key_management_system_keys::keys::ZkPublicKey;
use lb_key_management_system_keys::keys::{ZkKey, ZkPublicKey};
use lb_sdp_service::DeclarationConfig;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct StakeHolderInfo {
    pub zk_id: ZkPublicKey,
    pub stake: NoteValue,
}

/// `ConsensusExcludedKey` is used to register a consensus-excluded key with
/// its value.
#[derive(Clone, Debug, Deserialize)]
pub struct ConsensusExcludedKey {
    pub zk_id: ZkPublicKey,
    pub funds: NoteValue,
}

// TODO(RFC-480): the distribution is a list of genesis allocations, the
// builder derives the Transfer outputs from them.
// pub struct GenesisTransferOp {
//     transfer_op: TransferOp,
// }
//
// impl GenesisTransferOp {
//     pub fn new(
//         stake_holders: impl Iterator<Item = StakeHolderInfo>,
//         excluded_key: &ConsensusExcludedKey,
//     ) -> Self {
//         let mut notes: Vec<Note> = stake_holders
//             .map(|stake_holder| Note::new(stake_holder.stake, stake_holder.zk_id))
//             .collect();
//
//         notes.push(Note::new(excluded_key.funds, excluded_key.zk_id));
//
//         let outputs = Outputs::try_new(notes)
//             .expect("genesis distribution outputs must fit transfer output bounds");
//         let transfer_op = TransferOp::new(Inputs::empty(), outputs);
//
//         Self { transfer_op }
//     }
//
//     pub fn notes(&self) -> impl Iterator<Item = Note> {
//         self.transfer_op.utxos().map(|u| u.note)
//     }
//
//     pub fn utxos(&self) -> impl Iterator<Item = Utxo> {
//         self.transfer_op.utxos()
//     }
//
//     #[must_use]
//     pub fn utxo_by_index(&self, index: usize) -> Option<Utxo> {
//         self.transfer_op.utxo_by_index(index)
//     }
// }

#[derive(Error, Debug)]
pub enum DistributionError {
    #[error("Provider with ZK ID {0:?} is not a registered stakeholder")]
    ProviderNotStakeHolder(Box<DeclarationConfig>),

    #[error("Note already used for service {0:?}")]
    NoteAlreadyUsedForService(ServiceType),

    #[error("Provider {0:?} has no stake key")]
    MissingStakeKey(ZkPublicKey),
}

/// `distribute` stake to stake holders.
/// Provider has to be a stakeholder, because the declaration spends the
/// stakeholder note with the stake key of its `zk_id`.
#[expect(
    clippy::implicit_hasher,
    clippy::type_complexity,
    reason = "the stake keys file map and the distribution tuple"
)]
pub fn distribute<S, P>(
    stake_holders: S,
    providers: P,
    excluded_key: &ConsensusExcludedKey,
    // TODO(RFC-480): a declaration spends the stakeholder note with its stake key.
    // ) -> Result<(GenesisTransferOp, Vec<SDPDeclareOp>), DistributionError>
    stake_keys: &HashMap<ZkPublicKey, ZkKey>,
) -> Result<(Vec<GenesisAllocation>, Vec<SDPDeclareOp>, Vec<ZkKey>), DistributionError>
where
    S: IntoIterator<Item = StakeHolderInfo> + Clone,
    P: IntoIterator<Item = DeclarationConfig>,
{
    let stake_holder_keys: HashSet<ZkPublicKey> =
        stake_holders.clone().into_iter().map(|s| s.zk_id).collect();

    // TODO(RFC-480): the distribution is a list of genesis allocations.
    // let transfer_op = GenesisTransferOp::new(stake_holders.into_iter(), excluded_key);
    let allocations: Vec<GenesisAllocation> = stake_holders
        .into_iter()
        .map(|stake_holder| GenesisAllocation {
            value: stake_holder.stake,
            public_key: stake_holder.zk_id,
        })
        .chain([GenesisAllocation {
            value: excluded_key.funds,
            public_key: excluded_key.zk_id,
        }])
        .collect();
    let mut declarations = Vec::new();
    let mut keys = Vec::new();
    let mut used_services = HashSet::new();

    for provider in providers {
        if !stake_holder_keys.contains(&provider.zk_id) {
            return Err(DistributionError::ProviderNotStakeHolder(Box::new(
                provider,
            )));
        }

        if !used_services.insert((provider.zk_id, provider.service_type)) {
            return Err(DistributionError::NoteAlreadyUsedForService(
                provider.service_type,
            ));
        }

        // TODO(RFC-480): a declaration spends the stakeholder note with its stake key.
        // if let Some(utxo) = transfer_op.utxos().find(|u| u.note.pk == provider.zk_id) {
        //     declarations.push(SDPDeclareOp {
        //         service_type: provider.service_type,
        //         locators: provider.locators,
        //         provider_id: provider.provider_id,
        //         zk_id: provider.zk_id,
        //         service_note_id: utxo.id(),
        //     });
        // }
        if let Some(index) = allocations
            .iter()
            .position(|allocation| allocation.public_key == provider.zk_id)
        {
            let stake_key = stake_keys
                .get(&provider.zk_id)
                .ok_or(DistributionError::MissingStakeKey(provider.zk_id))?;
            let allocation = allocations[index];
            declarations.push(SDPDeclareOp {
                service_type: provider.service_type,
                locators: provider.locators,
                provider_id: provider.provider_id,
                zk_id: provider.zk_id,
                inputs: [derive_note_nf(
                    &allocation.note(index).commitment(),
                    stake_key.as_fr(),
                )]
                .into(),
                cm_merkle_root: CommitmentMmr::new().frontier_root(),
                amount: allocation.value,
            });
            keys.push(stake_key.clone());
        }
    }

    // TODO(RFC-480): a declaration spends the stakeholder note with its stake key.
    // Ok((transfer_op, declarations))
    Ok((allocations, declarations, keys))
}

#[cfg(test)]
mod tests {
    use lb_core::sdp::{Locator, ProviderId};
    use lb_groth16::Fr;
    use lb_key_management_system_keys::keys::Ed25519PublicKey;
    use num_bigint::BigUint;

    use super::*;

    fn mock_zk_pk(byte: u8) -> ZkPublicKey {
        ZkPublicKey::from(BigUint::from(byte))
    }

    fn mock_provider_id(byte: u8) -> ProviderId {
        ProviderId(Ed25519PublicKey::from_bytes(&[byte; 32]).unwrap())
    }

    #[test]
    fn declarations_spend_the_stakeholder_note_with_its_key() {
        let stake_key = ZkKey::new(Fr::from(5u64));
        let zk_id = stake_key.to_public_key();
        let stake_holders = vec![
            StakeHolderInfo {
                zk_id,
                stake: 1_000,
            },
            StakeHolderInfo {
                zk_id: mock_zk_pk(2),
                stake: 2_000,
            },
        ];
        let providers = vec![DeclarationConfig {
            provider_id: mock_provider_id(10),
            zk_id,
            locators: "/ip4/1.1.1.1/udp/0".parse::<Locator>().unwrap().into(),
            service_type: ServiceType::BlendNetwork,
        }];
        let excluded_key = ConsensusExcludedKey {
            zk_id: mock_zk_pk(3),
            funds: 100_000,
        };

        let (allocations, declarations, keys) = distribute(
            stake_holders,
            providers,
            &excluded_key,
            &HashMap::from([(zk_id, stake_key.clone())]),
        )
        .unwrap();
        assert_eq!(
            allocations[0],
            GenesisAllocation {
                value: 1_000,
                public_key: zk_id
            }
        );
        assert_eq!(
            declarations[0].inputs.as_slice(),
            [derive_note_nf(
                &allocations[0].note(0).commitment(),
                stake_key.as_fr()
            )]
        );
        assert_eq!(
            declarations[0].cm_merkle_root,
            CommitmentMmr::new().frontier_root()
        );
        assert_eq!(declarations[0].amount, 1_000);
        assert_eq!(keys, vec![stake_key]);
    }

    #[test]
    fn test_successful_distribution() {
        // TODO(RFC-480): a declaration spends the stakeholder note with its stake key.
        // let zk_id_1 = mock_zk_pk(1);
        let stake_key = ZkKey::new(Fr::from(1u64));
        let zk_id_1 = stake_key.to_public_key();
        let zk_id_2 = mock_zk_pk(2);

        let stake_holders = vec![
            StakeHolderInfo {
                zk_id: zk_id_1,
                stake: 1000,
            },
            StakeHolderInfo {
                zk_id: zk_id_2,
                stake: 2000,
            },
        ];

        let providers = vec![DeclarationConfig {
            provider_id: mock_provider_id(10),
            zk_id: zk_id_1,
            locators: "/ip4/1.1.1.1/udp/0".parse::<Locator>().unwrap().into(),
            service_type: ServiceType::BlendNetwork,
        }];

        let excluded_key = ConsensusExcludedKey {
            zk_id: mock_zk_pk(3),
            funds: 100_000,
        };

        // TODO(RFC-480): the distribution is a list of genesis allocations.
        // let result = distribute(stake_holders, providers, &excluded_key);
        //
        // assert!(result.is_ok());
        // let (transfer_op, declarations) = result.unwrap();
        // let notes = transfer_op.notes().collect::<Vec<_>>();
        //
        // assert_eq!(notes.len(), 3);
        // assert_eq!(notes[0].pk, zk_id_1);
        // assert_eq!(notes[1].pk, zk_id_2);
        //
        // assert_eq!(declarations.len(), 1);
        // assert_eq!(declarations[0].zk_id, zk_id_1);
        // assert_eq!(
        //     declarations[0].service_note_id,
        //     transfer_op.utxo_by_index(0).unwrap().id(),
        // );
        let result = distribute(
            stake_holders,
            providers,
            &excluded_key,
            &HashMap::from([(zk_id_1, stake_key.clone())]),
        );

        assert!(result.is_ok());
        let (allocations, declarations, keys) = result.unwrap();

        assert_eq!(allocations.len(), 3);
        assert_eq!(allocations[0].public_key, zk_id_1);
        assert_eq!(allocations[1].public_key, zk_id_2);
        assert_eq!(allocations[2].public_key, mock_zk_pk(3));

        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0].zk_id, zk_id_1);
        assert_eq!(keys, vec![stake_key]);
    }

    #[test]
    fn test_error_unauthorized_provider() {
        let stake_holders = vec![StakeHolderInfo {
            zk_id: mock_zk_pk(1),
            stake: 1000,
        }];

        let providers = vec![DeclarationConfig {
            provider_id: mock_provider_id(10),
            zk_id: mock_zk_pk(2),
            locators: "/ip4/1.1.1.1/udp/0".parse::<Locator>().unwrap().into(),
            service_type: ServiceType::BlendNetwork,
        }];

        let excluded_key = ConsensusExcludedKey {
            zk_id: mock_zk_pk(3),
            funds: 100_000,
        };

        // TODO(RFC-480): a declaration spends the stakeholder note with its stake key.
        // let result = distribute(stake_holders, providers, &excluded_key);
        let result = distribute(stake_holders, providers, &excluded_key, &HashMap::new());

        assert!(matches!(
            result,
            Err(DistributionError::ProviderNotStakeHolder(_))
        ));

        if let Err(DistributionError::ProviderNotStakeHolder(info)) = result {
            assert_eq!(info.zk_id, mock_zk_pk(2));
        }
    }

    #[test]
    fn test_error_already_used() {
        // TODO(RFC-480): a declaration spends the stakeholder note with its stake key.
        // let zk_id = mock_zk_pk(1);
        let stake_key = ZkKey::new(Fr::from(1u64));
        let zk_id = stake_key.to_public_key();
        let stake_holders = vec![StakeHolderInfo { zk_id, stake: 5000 }];

        // Two providers trying to use the same note for the same ServiceType.
        let providers = vec![
            DeclarationConfig {
                provider_id: mock_provider_id(10),
                zk_id,
                locators: "/ip4/1.1.1.1/udp/0".parse::<Locator>().unwrap().into(),
                service_type: ServiceType::BlendNetwork,
            },
            DeclarationConfig {
                provider_id: mock_provider_id(11),
                zk_id,
                locators: "/ip4/1.1.1.1/udp/0".parse::<Locator>().unwrap().into(),
                service_type: ServiceType::BlendNetwork,
            },
        ];

        let excluded_key = ConsensusExcludedKey {
            zk_id: mock_zk_pk(3),
            funds: 100_000,
        };

        // TODO(RFC-480): a declaration spends the stakeholder note with its stake key.
        // let result = distribute(stake_holders, providers, &excluded_key);
        let result = distribute(
            stake_holders,
            providers,
            &excluded_key,
            &HashMap::from([(zk_id, stake_key)]),
        );

        assert!(matches!(
            result,
            Err(DistributionError::NoteAlreadyUsedForService(
                ServiceType::BlendNetwork
            ))
        ));
    }

    #[test]
    fn test_error_missing_stake_key() {
        let zk_id = mock_zk_pk(1);
        let stake_holders = vec![StakeHolderInfo { zk_id, stake: 5000 }];
        let providers = vec![DeclarationConfig {
            provider_id: mock_provider_id(10),
            zk_id,
            locators: "/ip4/1.1.1.1/udp/0".parse::<Locator>().unwrap().into(),
            service_type: ServiceType::BlendNetwork,
        }];
        let excluded_key = ConsensusExcludedKey {
            zk_id: mock_zk_pk(3),
            funds: 100_000,
        };

        let result = distribute(stake_holders, providers, &excluded_key, &HashMap::new());

        assert!(matches!(
            result,
            Err(DistributionError::MissingStakeKey(key)) if key == zk_id
        ));
    }
}
