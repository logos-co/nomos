//! Password-manager data and operations.
//!
//! `PasswordManager` owns `LogosSql` directly so the example shows where the
//! application ends and the replication library begins. Domain operations are
//! translated into parameterized SQL writes here.

use logos_sql::{
    Displacement, DisplacementReason, Error as LogosSqlError, LogosSql, LogosSqlConfig,
    TransactionBuilder, TxId,
};
use rusqlite::OptionalExtension as _;

use crate::{AppError, AppResult, encryption::Encryption};

const SELECT_CREDENTIALS: &str = "
    SELECT label, account
    FROM credentials
    ORDER BY label
";

const SELECT_SCHEMA_EXISTS: &str = "
    SELECT COUNT(*) = 1
    FROM sqlite_schema
    WHERE type = 'table' AND name = 'credentials'
";

fn prepare_schema() -> TransactionBuilder {
    TransactionBuilder::new(
        "CREATE TABLE IF NOT EXISTS credentials (
            label TEXT PRIMARY KEY,
            account BLOB NOT NULL,
            password BLOB NOT NULL,
            notes BLOB NOT NULL
        )",
    )
}

fn prepare_credential_insert(credential: &StoredCredential) -> TransactionBuilder {
    TransactionBuilder::new(
        "INSERT INTO credentials (label, account, password, notes)
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&credential.label)
    .bind(credential.account.as_slice())
    .bind(credential.password.as_slice())
    .bind(credential.notes.as_slice())
}

fn prepare_password_update(
    label: &str,
    password: &[u8],
    old_password: &[u8],
) -> TransactionBuilder {
    TransactionBuilder::new(
        "UPDATE credentials SET password = ?2
         WHERE label = ?1 AND password = ?3",
    )
    .bind(label)
    .bind(password)
    .bind(old_password)
}

fn prepare_notes_update(label: &str, notes: &[u8], old_notes: &[u8]) -> TransactionBuilder {
    TransactionBuilder::new(
        "UPDATE credentials SET notes = ?2
         WHERE label = ?1 AND notes = ?3",
    )
    .bind(label)
    .bind(notes)
    .bind(old_notes)
}

fn prepare_credential_delete(credential: &StoredCredential) -> TransactionBuilder {
    TransactionBuilder::new(
        "DELETE FROM credentials
         WHERE label = ?1 AND account = ?2 AND password = ?3 AND notes = ?4",
    )
    .bind(&credential.label)
    .bind(credential.account.as_slice())
    .bind(credential.password.as_slice())
    .bind(credential.notes.as_slice())
}

/// One credential in the current local database view.
///
/// Fields are decrypted for display. Do not use this example with real
/// credentials.
#[derive(Debug)]
pub struct Credential {
    pub label: String,
    pub account: String,
    pub password: String,
    pub notes: String,
}

struct StoredCredential {
    label: String,
    account: Vec<u8>,
    password: Vec<u8>,
    notes: Vec<u8>,
}

impl StoredCredential {
    fn decrypt(&self, encryption: &Encryption) -> AppResult<Credential> {
        Ok(Credential {
            label: self.label.clone(),
            account: encryption.decrypt(&self.label, "account", &self.account)?,
            password: encryption.decrypt(&self.label, "password", &self.password)?,
            notes: encryption.decrypt(&self.label, "notes", &self.notes)?,
        })
    }
}

/// Credential labels and decrypted accounts shown when listing the database.
#[derive(Debug)]
pub struct CredentialSummary {
    pub label: String,
    pub account: String,
}

/// Keeps a displayed description paired with the exact displacement it
/// describes.
#[derive(PartialEq)]
pub struct DisplacementForReview {
    pub displacement: Displacement,
    pub description: String,
}

/// Application-facing password-manager operations over one `λSQL` database.
pub struct PasswordManager {
    logos_sql: LogosSql,
    encryption: Encryption,
}

impl PasswordManager {
    /// Starts `λSQL`, catches up with the channel, and prepares the database.
    pub async fn start(config: LogosSqlConfig, encryption: Encryption) -> AppResult<Self> {
        let manager = Self {
            logos_sql: LogosSql::start(config).await?,
            encryption,
        };

        match manager.initialize().await {
            // Keep the terminal available if schema creation was displaced.
            Ok(()) | Err(AppError::LogosSql(LogosSqlError::UnhandledDisplacements)) => {}
            Err(error) => {
                manager.shutdown().await?;
                return Err(error);
            }
        }

        Ok(manager)
    }

    async fn initialize(&self) -> AppResult<()> {
        if !self.schema_exists()? {
            self.logos_sql.execute(prepare_schema()).await?;

            return Ok(());
        }

        // Check the key against existing entries before accepting new writes.
        for credential in self.credentials()? {
            self.credential(&credential.label)?;
        }

        Ok(())
    }

    /// Existing participants install the schema through channel replay before
    /// startup completes. Only an empty database needs to publish the DDL.
    fn schema_exists(&self) -> Result<bool, LogosSqlError> {
        let connection = self.logos_sql.read_connection()?;

        Ok(connection.query_row(SELECT_SCHEMA_EXISTS, [], |row| row.get(0))?)
    }

    /// Adds one credential.
    ///
    /// Concurrent attempts to use the same label produce a primary-key
    /// conflict.
    pub async fn add(&self, label: String, account: String, password: String) -> AppResult<TxId> {
        let credential = StoredCredential {
            account: self.encryption.encrypt(&label, "account", &account)?,
            password: self.encryption.encrypt(&label, "password", &password)?,
            notes: self.encryption.encrypt(&label, "notes", "")?,
            label,
        };

        Ok(self
            .logos_sql
            .execute(prepare_credential_insert(&credential))
            .await?)
    }

    /// Replaces the password only if it still matches the value read here.
    pub async fn update_password(&self, label: String, password: String) -> AppResult<TxId> {
        let credential = self
            .stored_credential(&label)?
            .ok_or(AppError::CredentialNotFound)?;

        credential.decrypt(&self.encryption)?;
        let password = self.encryption.encrypt(&label, "password", &password)?;

        Ok(self
            .logos_sql
            .execute(prepare_password_update(
                &label,
                &password,
                &credential.password,
            ))
            .await?)
    }

    /// Updates notes independently of password changes.
    pub async fn update_notes(&self, label: String, notes: String) -> AppResult<TxId> {
        let credential = self
            .stored_credential(&label)?
            .ok_or(AppError::CredentialNotFound)?;

        credential.decrypt(&self.encryption)?;
        let notes = self.encryption.encrypt(&label, "notes", &notes)?;

        Ok(self
            .logos_sql
            .execute(prepare_notes_update(&label, &notes, &credential.notes))
            .await?)
    }

    /// Removes a credential only if all its values still match the read.
    pub async fn remove(&self, label: String) -> AppResult<TxId> {
        let credential = self
            .stored_credential(&label)?
            .ok_or(AppError::CredentialNotFound)?;

        credential.decrypt(&self.encryption)?;

        Ok(self
            .logos_sql
            .execute(prepare_credential_delete(&credential))
            .await?)
    }

    /// Reads one credential from the local `SQLite` database.
    pub fn credential(&self, label: &str) -> AppResult<Option<Credential>> {
        self.stored_credential(label)?
            .map(|credential| credential.decrypt(&self.encryption))
            .transpose()
    }

    fn stored_credential(&self, label: &str) -> AppResult<Option<StoredCredential>> {
        if !self.schema_exists()? {
            return Ok(None);
        }

        let connection = self.logos_sql.read_connection()?;

        let mut statement = connection.prepare(
            "SELECT label, account, password, notes
             FROM credentials WHERE label = ?1",
        )?;

        Ok(statement
            .query_row([label], |row| {
                Ok(StoredCredential {
                    label: row.get("label")?,
                    account: row.get("account")?,
                    password: row.get("password")?,
                    notes: row.get("notes")?,
                })
            })
            .optional()?)
    }

    /// Lists credential labels and accounts from the local database.
    pub fn credentials(&self) -> AppResult<Vec<CredentialSummary>> {
        if !self.schema_exists()? {
            return Ok(Vec::new());
        }

        let connection = self.logos_sql.read_connection()?;
        let mut statement = connection.prepare(SELECT_CREDENTIALS)?;

        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>("label")?,
                row.get::<_, Vec<u8>>("account")?,
            ))
        })?;

        rows.map(|row| {
            let (label, stored) = row?;
            let account = self.encryption.decrypt(&label, "account", &stored)?;

            Ok(CredentialSummary { label, account })
        })
        .collect()
    }

    /// Retries the saved SQL with its original conditions, not new reads.
    pub async fn retry_displacement(&self, displacement: &Displacement) -> AppResult<TxId> {
        Ok(self.logos_sql.retry_displacement(displacement).await?)
    }

    /// Marks one reviewed displacement handled without resubmitting it.
    pub async fn handle_displacement(&self, displacement: Displacement) -> AppResult<()> {
        self.logos_sql
            .mark_displacement_handled(displacement)
            .await?;

        Ok(())
    }

    /// Lists displacements awaiting review without printing SQL or passwords.
    pub async fn displacements(&self) -> AppResult<Vec<DisplacementForReview>> {
        let mut displacements = Vec::new();

        for displacement in self.logos_sql.unhandled_displacements().await? {
            let reason = match displacement.reason {
                DisplacementReason::Orphaned => "removed from channel history",
                DisplacementReason::PendingWriteInvalidated => {
                    "channel history changed before publication completed"
                }
            };
            let description = format!("{}: {reason}; awaiting review", displacement.tx_id);

            displacements.push(DisplacementForReview {
                displacement,
                description,
            });
        }

        Ok(displacements)
    }

    /// Gracefully stops the owned `λSQL` runtime.
    pub async fn shutdown(self) -> Result<(), LogosSqlError> {
        self.logos_sql.shutdown().await
    }
}
