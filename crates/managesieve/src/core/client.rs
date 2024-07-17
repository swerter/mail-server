/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs Ltd <hello@stalw.art>
 *
 * SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-SEL
 */

use common::listener::{SessionResult, SessionStream};
use imap_proto::receiver::{self, Request};
use jmap_proto::types::{collection::Collection, property::Property};
use store::query::Filter;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use trc::AddContext;

use super::{Command, ResponseCode, SerializeResponse, Session, State, StatusResponse};

impl<T: SessionStream> Session<T> {
    pub async fn ingest(&mut self, bytes: &[u8]) -> SessionResult {
        /*let tmp = "dd";
        for line in String::from_utf8_lossy(bytes).split("\r\n") {
            println!("<- {:?}", &line[..std::cmp::min(line.len(), 100)]);
        }*/

        let mut bytes = bytes.iter();
        let mut requests = Vec::with_capacity(2);
        let mut needs_literal = None;

        loop {
            match self.receiver.parse(&mut bytes) {
                Ok(request) => match self.validate_request(request).await {
                    Ok(request) => {
                        requests.push(request);
                    }
                    Err(err) => {
                        if let Err(err) = self.write_error(err).await {
                            tracing::error!(parent: &self.span, event = "error", error = ?err);
                            return SessionResult::Close;
                        }
                    }
                },
                Err(receiver::Error::NeedsMoreData) => {
                    break;
                }
                Err(receiver::Error::NeedsLiteral { size }) => {
                    needs_literal = size.into();
                    break;
                }
                Err(receiver::Error::Error { response }) => {
                    if let Err(err) = self
                        .write(&StatusResponse::no(response.message).into_bytes())
                        .await
                    {
                        tracing::error!(parent: &self.span, event = "error", error = ?err);
                        return SessionResult::Close;
                    }
                    break;
                }
            }
        }

        for request in requests {
            let command = request.command;
            match match command {
                Command::ListScripts => self.handle_listscripts().await,
                Command::PutScript => self.handle_putscript(request).await,
                Command::SetActive => self.handle_setactive(request).await,
                Command::GetScript => self.handle_getscript(request).await,
                Command::DeleteScript => self.handle_deletescript(request).await,
                Command::RenameScript => self.handle_renamescript(request).await,
                Command::CheckScript => self.handle_checkscript(request).await,
                Command::HaveSpace => self.handle_havespace(request).await,
                Command::Capability => self.handle_capability("").await,
                Command::Authenticate => self.handle_authenticate(request).await,
                Command::StartTls => self.handle_start_tls().await,
                Command::Logout => self.handle_logout().await,
                Command::Noop => self.handle_noop(request).await,
                Command::Unauthenticate => self.handle_unauthenticate().await,
            } {
                Ok(response) => {
                    if let Err(err) = self.write(&response).await {
                        tracing::error!(parent: &self.span, event = "error", error = ?err);
                        return SessionResult::Close;
                    }

                    match command {
                        Command::Logout => return SessionResult::Close,
                        Command::StartTls => return SessionResult::UpgradeTls,
                        _ => (),
                    }
                }
                Err(err) => {
                    if let Err(err) = self.write_error(err).await {
                        tracing::error!(parent: &self.span, event = "error", error = ?err);
                        return SessionResult::Close;
                    }
                }
            }
        }

        if let Some(needs_literal) = needs_literal {
            if let Err(err) = self
                .write(format!("OK Ready for {} bytes.\r\n", needs_literal).as_bytes())
                .await
            {
                tracing::error!(parent: &self.span, event = "error", error = ?err);
                return SessionResult::Close;
            }
        }

        SessionResult::Continue
    }

    async fn validate_request(&self, command: Request<Command>) -> trc::Result<Request<Command>> {
        match &command.command {
            Command::Capability | Command::Logout | Command::Noop => Ok(command),
            Command::Authenticate => {
                if let State::NotAuthenticated { .. } = &self.state {
                    if self.stream.is_tls() || self.jmap.core.imap.allow_plain_auth {
                        Ok(command)
                    } else {
                        Err(trc::Cause::ManageSieve
                            .into_err()
                            .code(ResponseCode::EncryptNeeded)
                            .details("Cannot authenticate over plain-text."))
                    }
                } else {
                    Err(trc::Cause::ManageSieve
                        .into_err()
                        .details("Already authenticated."))
                }
            }
            Command::StartTls => {
                if !self.stream.is_tls() {
                    Ok(command)
                } else {
                    Err(trc::Cause::ManageSieve
                        .into_err()
                        .details("Already in TLS mode."))
                }
            }
            Command::HaveSpace
            | Command::PutScript
            | Command::ListScripts
            | Command::SetActive
            | Command::GetScript
            | Command::DeleteScript
            | Command::RenameScript
            | Command::CheckScript
            | Command::Unauthenticate => {
                if let State::Authenticated { access_token, .. } = &self.state {
                    if let Some(rate) = &self.jmap.core.imap.rate_requests {
                        if self
                            .jmap
                            .core
                            .storage
                            .lookup
                            .is_rate_allowed(
                                format!("ireq:{}", access_token.primary_id()).as_bytes(),
                                rate,
                                true,
                            )
                            .await
                            .caused_by(trc::location!())?
                            .is_none()
                        {
                            Ok(command)
                        } else {
                            Err(trc::LimitCause::TooManyRequests
                                .into_err()
                                .code(ResponseCode::TryLater))
                        }
                    } else {
                        Ok(command)
                    }
                } else {
                    Err(trc::Cause::ManageSieve
                        .into_err()
                        .details("Not authenticated."))
                }
            }
        }
    }
}

impl<T: AsyncWrite + AsyncRead + Unpin> Session<T> {
    #[inline(always)]
    pub async fn write(&mut self, bytes: &[u8]) -> trc::Result<()> {
        self.stream
            .write_all(bytes)
            .await
            .map_err(|err| trc::Cause::Network.reason(err).caused_by(trc::location!()))?;
        self.stream
            .flush()
            .await
            .map_err(|err| trc::Cause::Network.reason(err).caused_by(trc::location!()))?;

        tracing::trace!(parent: &self.span,
            event = "write",
            data = std::str::from_utf8(bytes).unwrap_or_default() ,
            size = bytes.len());

        Ok(())
    }

    pub async fn write_error(&mut self, error: trc::Error) -> trc::Result<()> {
        tracing::error!(parent: &self.span, event = "error", error = ?error);
        self.write(&error.serialize()).await
    }

    #[inline(always)]
    pub async fn read(&mut self, bytes: &mut [u8]) -> trc::Result<usize> {
        let len = self
            .stream
            .read(bytes)
            .await
            .map_err(|err| trc::Cause::Network.reason(err).caused_by(trc::location!()))?;

        tracing::trace!(parent: &self.span,
            event = "read",
            data =  bytes
                .get(0..len)
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .unwrap_or("[invalid UTF8]"),
            size = len);

        Ok(len)
    }
}

impl<T: AsyncWrite + AsyncRead> Session<T> {
    pub async fn get_script_id(&self, account_id: u32, name: &str) -> trc::Result<u32> {
        self.jmap
            .core
            .storage
            .data
            .filter(
                account_id,
                Collection::SieveScript,
                vec![Filter::eq(Property::Name, name)],
            )
            .await
            .caused_by(trc::location!())
            .and_then(|results| {
                results.results.min().ok_or_else(|| {
                    trc::Cause::ManageSieve
                        .into_err()
                        .code(ResponseCode::NonExistent)
                        .reason("There is no script by that name")
                })
            })
    }
}
