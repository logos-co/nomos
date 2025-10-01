use overwatch::services::ServiceData;

use crate::{WalletMsg, WalletServiceSettings};

pub trait WalletServiceData:
    ServiceData<Settings = WalletServiceSettings, Message = WalletMsg>
{
}

impl<T> WalletServiceData for T where
    T: ServiceData<Settings = WalletServiceSettings, Message = WalletMsg>
{
}

pub trait WalletApi: WalletServiceData {
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
