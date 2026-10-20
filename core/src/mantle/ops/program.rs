use lb_binary_codec::canonical::{BinaryCodec, BinaryEncode as _};
use lb_groth16::KeyPointError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    events::TxEvent,
    mantle::{
        gas::{Gas, MainnetGasProfile, OpGasCalculator, OperationGas},
        ledger::{
            ExecutableOperation, PreverifiableOperation, ProvableOperation, VerifiableOperation,
            verification_mode::StandardMode,
        },
        ops::{NoOpProof, OpId, SignedOperation},
        programs::{ProgramVerificationKey, Programs, RegisteredProgram},
        transactions::states::{Preverified, Unverified, Verified},
    },
};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, BinaryCodec)]
pub struct ProgramRegisterOp {
    pub verification_key: ProgramVerificationKey,
}

impl ProgramRegisterOp {
    #[cfg(any(test, feature = "test-utils"))]
    #[must_use]
    pub const fn sample() -> Self {
        Self {
            verification_key: ProgramVerificationKey {
                alpha: [0x3e; 32],
                beta: [0x3f; 64],
                gamma: [0x40; 64],
                delta: [0x41; 64],
                ic: [[0x42; 32], [0x43; 32], [0x44; 32], [0x45; 32]],
            },
        }
    }
}

impl OpId for ProgramRegisterOp {
    fn op_bytes(&self) -> Vec<u8> {
        self.encode_to_vec()
    }
}

impl ProvableOperation for ProgramRegisterOp {
    type Proof = NoOpProof;
    const CODE: u8 = 0x01;
}

impl OperationGas<MainnetGasProfile> for ProgramRegisterOp {
    const GAS_COST: Gas = Gas::new(1896);
}

impl OpGasCalculator<MainnetGasProfile> for ProgramRegisterOp {}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ProgramRegisterError {
    #[error("Invalid program verification key: {0}")]
    InvalidKey(#[from] KeyPointError),
}

pub struct ProgramRegisterExecutionContext {
    pub programs: Programs,
}

impl PreverifiableOperation<StandardMode>
    for SignedOperation<ProgramRegisterOp, Unverified, StandardMode>
{
    type Context<'a> = ();
    type Error = ProgramRegisterError;

    fn preverify(&self, _context: &Self::Context<'_>) -> Result<(), Self::Error> {
        // Every point on its curve, in its prime order subgroup and not the point
        // at infinity, since an invalid point would make the proofs of the program
        // unsound.
        self.operation()
            .verification_key
            .to_groth16()
            .map(drop)
            .map_err(Into::into)
    }
}

impl VerifiableOperation<StandardMode>
    for SignedOperation<ProgramRegisterOp, Preverified, StandardMode>
{
    type DeferredProof = ();
    type Context<'a> = ();
    type Error = ProgramRegisterError;

    fn verify(&self, _context: &Self::Context<'_>) -> Result<Self::DeferredProof, Self::Error> {
        Ok(())
    }
}

impl ExecutableOperation for SignedOperation<ProgramRegisterOp, Verified, StandardMode> {
    type Context<'a> = ProgramRegisterExecutionContext;
    type Error = ProgramRegisterError;

    fn execute<'a>(
        &self,
        context: Self::Context<'a>,
    ) -> Result<(Self::Context<'a>, Vec<TxEvent>), Self::Error> {
        // Storing an already registered key has no effect, and e(α, β) is computed
        // once here, paid by the registration.
        let program = RegisteredProgram::new(self.operation().verification_key)?;
        let (programs, _) = context.programs.register(program);
        Ok((ProgramRegisterExecutionContext { programs }, Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use lb_groth16::{
        KeyPoint,
        test_utils::{off_subgroup_g2_compressed, three_input_setup},
    };

    use super::*;
    use crate::mantle::{gas::test_utils::FixedThresholds, programs::derive_program_id};

    fn valid_key() -> ProgramVerificationKey {
        ProgramVerificationKey::try_from(&three_input_setup(1).0).unwrap()
    }

    fn preverify(verification_key: &ProgramVerificationKey) -> Result<(), ProgramRegisterError> {
        SignedOperation::<_, Unverified, StandardMode>::new(
            ProgramRegisterOp {
                verification_key: *verification_key,
            },
            NoOpProof,
        )
        .preverify(&())
    }

    #[test]
    fn preverify_checks_every_point_of_the_key() {
        assert_eq!(
            preverify(&ProgramRegisterOp::sample().verification_key),
            Err(ProgramRegisterError::InvalidKey(KeyPointError::Invalid(
                KeyPoint::Alpha
            )))
        );
        let key = valid_key();
        assert_eq!(
            preverify(&ProgramVerificationKey {
                gamma: off_subgroup_g2_compressed(),
                ..key
            }),
            Err(ProgramRegisterError::InvalidKey(KeyPointError::Invalid(
                KeyPoint::Gamma
            )))
        );
        let mut infinity = [0u8; 64];
        infinity[63] = 0x40;
        assert_eq!(
            preverify(&ProgramVerificationKey {
                delta: infinity,
                ..key
            }),
            Err(ProgramRegisterError::InvalidKey(KeyPointError::Infinity(
                KeyPoint::Delta
            )))
        );
        assert_eq!(preverify(&key), Ok(()));
    }

    #[test]
    fn execute_registers_the_key_under_its_id_once() {
        let key = valid_key();
        let signed: SignedOperation<_, Verified, StandardMode> = SignedOperation::new(
            ProgramRegisterOp {
                verification_key: key,
            },
            NoOpProof,
        )
        .into_state_trusted();
        let (context, events) = signed
            .clone()
            .execute(ProgramRegisterExecutionContext {
                programs: Programs::new(),
            })
            .expect("a checked key registers");
        assert_eq!(events, []);
        assert_eq!(
            context
                .programs
                .get(&derive_program_id(&key))
                .map(RegisteredProgram::key),
            Some(&key)
        );
        let (again, _) = signed
            .execute(ProgramRegisterExecutionContext {
                programs: context.programs.clone(),
            })
            .expect("registering again has no effect");
        assert_eq!(again.programs, context.programs);
    }

    #[test]
    fn program_register_op_execution_gas_is_1896() {
        assert_eq!(
            ProgramRegisterOp::sample().execution_gas(&FixedThresholds(3)),
            Ok(Gas::new(1_896))
        );
    }
}
