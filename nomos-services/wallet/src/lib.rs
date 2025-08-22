Wallet design
Integration
 - support the wallet standard
 - integrate with Key Management service?
   - Wallet Standard implies sk's are stored in the wallet, should they also be stored in the KMS?
 - update note state based on cryptarchia state (note spent or received in mantle is reflected in wallet)
   - Needs to deal with reorgs gracefully where a note may become unspent if the block in which it's spent is reorged.
Functionality
 - Implements the Wallet Standard
 - Persistance (save keys and notes to disk)
 - Ability to spend notes, gracefully recovering from reorgs, support for double signing in the case where a note is used in an impossible transaction
 - Ability to receive notes (either automatically from observing the blockchain or by manually inserting notes in the wallet)
 - transaction signing using note secret keys.e
API
 - keygen
 - loading wallet from disk
 - spending notes
   - spent notes can't be immediately removed from the wallet. But they should be marked as spent but waiting confirmation.
 - recieve notes
   - when we receive a mantle block, we should know if we received a note or not.
   - we should know the set of addresses that we are recieving for.
   - e.g. when requesting a transfer, we should keep track of address we requested here.
 - signing of mantle transactions and by extension, approving the ledger transaction
   - API should take an unpayed for mantle transaction and then builds the ledger transaction that will pay for the mantle transaction fees.
 - API to build a ledger transaction?


pub struct Wallet<B: WalletBackend> {
    tip: HeaderId,
    backend: B,
}

impl<B: WalletBackend> Wallet<B {
    pub fn load(config: B::Config) -> Self {
        Self {
            backend: B::load(config)
        }
    }
}

pub trait WalletBackend {
    type Config;

    fn load(config: Config) -> Self;
}
