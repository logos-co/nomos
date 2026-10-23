use std::process::ExitCode;

use logos_blockchain_sql_tests::cucumber::run;

#[tokio::main]
async fn main() -> ExitCode {
    run().await
}
