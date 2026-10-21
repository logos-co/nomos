use std::io::Error as IoError;

use lb_common_http_client::Error as HttpError;
use lb_testing_framework::configs::wallet::WalletConfigError;
use logos_blockchain_tests::cucumber::error::StepError;
use logos_sql::Error as SqlError;
use rusqlite::Error as SqliteError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Harness(#[from] StepError),
    #[error(transparent)]
    LogosSql(#[from] SqlError),
    #[error(transparent)]
    Sqlite(#[from] SqliteError),
    #[error(transparent)]
    Io(#[from] IoError),
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error(transparent)]
    WalletConfig(#[from] WalletConfigError),
}

pub type StepResult<T = ()> = Result<T, Error>;
