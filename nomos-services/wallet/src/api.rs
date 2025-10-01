use overwatch::services::ServiceData;

pub trait WalletApi: ServiceData {
    type Cryptarchia;
    type Tx;
    type Storage;
}

impl<Cryptarchia, Tx, Storage, RuntimeServiceId> WalletApi
    for crate::WalletService<Cryptarchia, Tx, Storage, RuntimeServiceId>
{
    type Cryptarchia = Cryptarchia;
    type Tx = Tx;
    type Storage = Storage;
}
