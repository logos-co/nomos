use super::{GasConstants, GasCost as _, MantleTx, Note, Op, Utxo, keys::PublicKey};
use crate::mantle::ledger::Tx as LedgerTx;

#[derive(Debug, Clone)]
pub struct MantleTxBuilder {
    mantle_tx: MantleTx,
    ledger_inputs: Vec<Utxo>,
}

impl MantleTxBuilder {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mantle_tx: MantleTx {
                ops: vec![],
                ledger_tx: LedgerTx {
                    inputs: vec![],
                    outputs: vec![],
                },
                execution_gas_price: 0,
                storage_gas_price: 0,
            },
            ledger_inputs: vec![],
        }
    }

    #[must_use]
    pub fn push_op(self, op: Op) -> Self {
        self.extend_ops([op])
    }

    #[must_use]
    pub fn extend_ops(mut self, ops: impl IntoIterator<Item = Op>) -> Self {
        self.mantle_tx.ops.extend(ops);
        self
    }

    #[must_use]
    pub fn add_ledger_input(self, utxo: Utxo) -> Self {
        self.extend_ledger_inputs([utxo])
    }

    #[must_use]
    pub fn extend_ledger_inputs(mut self, utxos: impl IntoIterator<Item = Utxo>) -> Self {
        for utxo in utxos {
            self.mantle_tx.ledger_tx.inputs.push(utxo.id());
            self.ledger_inputs.push(utxo);
        }
        self
    }

    #[must_use]
    pub fn add_ledger_output(self, note: Note) -> Self {
        self.extend_ledger_outputs([note])
    }

    #[must_use]
    pub fn extend_ledger_outputs(mut self, notes: impl IntoIterator<Item = Note>) -> Self {
        self.mantle_tx.ledger_tx.outputs.extend(notes);
        self
    }

    #[must_use]
    pub const fn set_execution_gas_price(mut self, price: u64) -> Self {
        self.mantle_tx.execution_gas_price = price;
        self
    }

    #[must_use]
    pub const fn set_storage_gas_price(mut self, price: u64) -> Self {
        self.mantle_tx.storage_gas_price = price;
        self
    }

    #[must_use]
    pub fn return_change<G: GasConstants>(self, change_reciever: PublicKey) -> Self {
        let delta = self.funding_delta::<G>();
        assert!(
            delta > 0,
            "It's assumed that that the tx has enough funds to merit a change note"
        );

        let change = u64::try_from(delta).expect("Positive funding delta must fit in u64");

        self.add_ledger_output(Note {
            value: change,
            pk: change_reciever,
        })
    }

    #[must_use]
    pub fn with_dummy_change_note(&self) -> Self {
        self.clone().add_ledger_output(Note {
            value: 0,
            pk: PublicKey::zero(),
        })
    }

    #[must_use]
    pub fn net_balance(&self) -> i128 {
        let in_sum: i128 = self
            .ledger_inputs
            .iter()
            .map(|utxo| i128::from(utxo.note.value))
            .sum();

        let out_sum: i128 = self
            .mantle_tx
            .ledger_tx
            .outputs
            .iter()
            .map(|n| i128::from(n.value))
            .sum();

        in_sum - out_sum
    }

    #[must_use]
    pub fn gas_cost<G: GasConstants>(&self) -> u64 {
        self.mantle_tx.gas_cost::<G>()
    }

    #[must_use]
    pub fn funding_delta<G: GasConstants>(&self) -> i128 {
        self.net_balance() - i128::from(self.gas_cost::<G>())
    }

    #[must_use]
    pub fn build(self) -> MantleTx {
        self.mantle_tx
    }
}

impl Default for MantleTxBuilder {
    fn default() -> Self {
        Self::new()
    }
}
