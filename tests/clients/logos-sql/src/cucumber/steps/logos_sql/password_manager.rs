//! Runs the password-manager executable, so scenarios use its actual SQL
//! builders.

use std::{
    env, fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::Duration,
};

use cucumber::{gherkin::Step, when};
use lb_groth16::fr_to_bytes;
use logos_blockchain_tests::cucumber::{error::StepError, steps::parse_steps::parse_table_rows};
use tokio::{
    io::AsyncWriteExt as _,
    process::{Child, Command},
    time::timeout,
};

use crate::cucumber::{error::StepResult, world::SqlWorld};

#[when(expr = "the password manager using sequencer {string} runs this session in {int} seconds:")]
#[expect(
    clippy::needless_pass_by_ref_mut,
    reason = "Cucumber requires a mutable world for step functions"
)]
async fn run_session(
    world: &mut SqlWorld,
    step: &Step,
    sequencer: String,
    seconds: u64,
) -> StepResult {
    let session = Session::from_step(step)?;
    let directory = app_directory(world);
    fs::create_dir_all(&directory)?;

    let child = start_password_manager(world, &sequencer, &directory)?;
    let output = session.run(child, Duration::from_secs(seconds)).await?;

    fs::write(directory.join("stdout.log"), &output.stdout)?;
    fs::write(directory.join("stderr.log"), &output.stderr)?;

    Ok(session.check_output(&output)?)
}

fn app_directory(world: &SqlWorld) -> PathBuf {
    // Keep restarts in one scenario together, but never reuse an earlier run's
    // database.
    let run = world
        .sdk
        .node
        .lifecycle
        .test_context
        .as_ref()
        .expect("scenario has a test context");

    world
        .sdk
        .node
        .lifecycle
        .scenario_base_dir
        .join("password_manager")
        .join(run)
}

/// The terminal commands and expected responses from the scenario table.
struct Session {
    commands: Vec<(String, String)>,
}

impl Session {
    fn from_step(step: &Step) -> Result<Self, StepError> {
        let commands = parse_table_rows(
            step,
            &["command", "output"],
            "password-manager session",
            |row| match row {
                [command, output] => Ok((command.clone(), output.clone())),
                _ => Err(StepError::InvalidArgument {
                    message: "password-manager session requires command and output columns"
                        .to_owned(),
                }),
            },
        )?;

        Ok(Self { commands })
    }

    async fn run(&self, mut child: Child, duration: Duration) -> Result<Output, StepError> {
        let mut input = self
            .commands
            .iter()
            .map(|(command, _)| command.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        input.push_str("\nexit\n");
        let mut stdin = child.stdin.take().ok_or_else(|| StepError::LogicalError {
            message: "password-manager stdin was not piped".to_owned(),
        })?;

        Ok(timeout(duration, async move {
            stdin.write_all(input.as_bytes()).await?;
            drop(stdin);
            child.wait_with_output().await
        })
        .await
        .map_err(|_| StepError::Timeout {
            message: "password-manager session did not finish".to_owned(),
        })??)
    }

    fn check_output(&self, output: &Output) -> Result<(), StepError> {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !output.status.success() || stderr.contains("error:") {
            return Err(StepError::StepFail {
                message: format!("password-manager session failed\n{stdout}\n{stderr}"),
            });
        }

        let responses = stdout
            .split("password-manager> ")
            .skip(1)
            .collect::<Vec<_>>();
        for (index, (command, expected)) in self.commands.iter().enumerate() {
            let actual = responses.get(index).copied().unwrap_or_default();
            if !actual.contains(expected) {
                return Err(StepError::StepFail {
                    message: format!(
                        "command `{command}`: expected `{expected}`, got `{actual}`\n{stderr}"
                    ),
                });
            }
        }

        Ok(())
    }
}

fn start_password_manager(
    world: &SqlWorld,
    sequencer: &str,
    directory: &Path,
) -> Result<Child, StepError> {
    let binary = env::var_os("PASSWORD_MANAGER_BIN").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../target/debug/examples/password_manager")
        },
        PathBuf::from,
    );
    if !binary.is_file() {
        return Err(StepError::Preflight {
            message: "Build `cargo build -p logos-sql --example password_manager`, or set PASSWORD_MANAGER_BIN".to_owned(),
        });
    }

    let node = world.sdk.zone.sequencer_node_name(sequencer)?;
    let funding_key = world.sdk.node.funding_wallet(node)?.public_key()?;
    let signing_key = world
        .sdk
        .zone
        .sequencer_signing_key(sequencer)?
        .clone()
        .into_unsecured();
    let channel = world.sdk.zone.sequencer_channel_id(sequencer)?;
    let key_path = directory.join("encryption.key");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;

        options.mode(0o600)
    };

    // A fixed test key lets subsequent sessions read the same credentials.
    options.open(&key_path)?.write_all(&[42; 32])?;

    Ok(Command::new(binary)
        .env("LOGOS_SQL_ENCRYPTION_KEY_FILE", key_path)
        .env("LOGOS_SQL_CHANNEL_ID", hex::encode(channel.as_ref()))
        .env("LOGOS_SQL_SIGNING_KEY", hex::encode(signing_key.as_bytes()))
        .env(
            "LOGOS_SQL_FUNDING_KEY",
            hex::encode(fr_to_bytes(&funding_key.into())),
        )
        .env("LOGOS_SQL_MAX_TX_FEE", u64::MAX.to_string())
        .env(
            "LOGOS_SQL_NODE_URL",
            world.sdk.zone_node_url_for_sequencer(sequencer)?.as_str(),
        )
        .env("LOGOS_SQL_STATE_DIR", directory.join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?)
}
