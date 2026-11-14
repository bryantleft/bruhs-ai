//! Purpose-bound password use. Callers receive a connected client, never a saved value.
use super::SecretStore;
use crate::database::{password_name, pgwire::BudgetStream, DatabaseError};
use bruhs_models::database::{DatabaseConnection, DatabaseTarget};
use std::sync::{atomic::AtomicBool, Arc};
use tokio::net::TcpStream;
use tokio_postgres::{config::SslMode, Client, NoTls};

pub(crate) struct DatabaseClient {
    pub client: Client,
    driver: tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
}
impl Drop for DatabaseClient {
    fn drop(&mut self) {
        self.driver.abort();
    }
}
impl SecretStore {
    pub(crate) async fn database_client(
        &self,
        connection: &DatabaseConnection,
        cancelled: Arc<AtomicBool>,
    ) -> Result<DatabaseClient, DatabaseError> {
        let DatabaseTarget::Postgres {
            host,
            port,
            database,
            user,
        } = &connection.target
        else {
            return Err(DatabaseError::Changed);
        };
        let addresses = tokio::net::lookup_host((host.as_str(), *port))
            .await
            .map_err(|_| {
                DatabaseError::Invalid("Could not resolve the local Postgres host.".into())
            })?;
        let mut last = None;
        let mut socket = None;
        for address in addresses {
            if !address.ip().is_loopback() {
                return Err(DatabaseError::Invalid(
                    "Postgres connections must resolve to loopback.".into(),
                ));
            }
            match TcpStream::connect(address).await {
                Ok(value) => {
                    socket = Some(value);
                    break;
                }
                Err(error) => last = Some(error),
            }
        }
        let socket = socket.ok_or_else(|| {
            DatabaseError::Query(format!(
                "Could not connect to local Postgres: {}",
                last.map_or_else(|| "no loopback address".into(), |error| error.to_string())
            ))
        })?;
        let mut config = tokio_postgres::Config::new();
        config
            .user(user)
            .dbname(database)
            .application_name("bruhs read-only browser")
            .ssl_mode(SslMode::Disable)
            .options("-c standard_conforming_strings=on");
        if connection.password_saved {
            let password=self.find(&connection.project_path,&password_name(connection.id,connection.credential_version)).await
                .map_err(|_| DatabaseError::Storage("Could not unlock the saved database password. Unlock the OS credential store and retry.".into()))?
                .ok_or_else(||DatabaseError::Storage("The database password is unavailable. Save it again in Databases.".into()))?;
            config.password(password.as_bytes());
        }
        let (client,driver)=config.connect_raw(BudgetStream::new(socket,cancelled),NoTls).await
            .map_err(|error|DatabaseError::Query(if error.as_db_error().is_some_and(|error|error.code().code().starts_with("28")){
                "Postgres authentication failed. Check the database, user, saved password, and server authentication settings."
            }else{
                "The local Postgres session could not be established within the query budget. Check that the server is running and the host/port are reachable, then retry."
            }.into()))?;
        Ok(DatabaseClient {
            client,
            driver: tokio::spawn(driver),
        })
    }
}
