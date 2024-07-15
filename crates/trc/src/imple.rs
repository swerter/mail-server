/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs Ltd <hello@stalw.art>
 *
 * SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-SEL
 */

use std::{borrow::Cow, fmt::Display};

use crate::*;

impl<T, const N: usize> Context<T, N>
where
    [(Key, Value); N]: Default,
    T: Eq,
{
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            keys: Default::default(),
            keys_size: 0,
        }
    }

    #[inline(always)]
    pub fn ctx(mut self, key: Key, value: impl Into<Value>) -> Self {
        if self.keys_size < N {
            self.keys[self.keys_size] = (key, value.into());
            self.keys_size += 1;
        } else {
            #[cfg(debug_assertions)]
            panic!(
                "Context is full while inserting {:?}: {:?}",
                key,
                value.into()
            );
        }
        self
    }

    pub fn ctx_opt(self, key: Key, value: Option<impl Into<Value>>) -> Self {
        match value {
            Some(value) => self.ctx(key, value),
            None => self,
        }
    }

    #[inline(always)]
    pub fn matches(&self, inner: T) -> bool {
        self.inner == inner
    }

    pub fn value(&self, key: Key) -> Option<&Value> {
        self.keys.iter().take(self.keys_size).find_map(
            |(k, v)| {
                if *k == key {
                    Some(v)
                } else {
                    None
                }
            },
        )
    }

    pub fn value_as_str(&self, key: Key) -> Option<&str> {
        self.value(key).and_then(|v| v.as_str())
    }

    pub fn take_value(&mut self, key: Key) -> Option<Value> {
        self.keys
            .iter_mut()
            .take(self.keys_size)
            .find_map(|(k, v)| {
                if *k == key {
                    Some(std::mem::take(v))
                } else {
                    None
                }
            })
    }

    #[inline(always)]
    pub fn caused_by(self, error: impl Into<Value>) -> Self {
        self.ctx(Key::CausedBy, error)
    }

    #[inline(always)]
    pub fn details(self, error: impl Into<Value>) -> Self {
        self.ctx(Key::Details, error)
    }

    #[inline(always)]
    pub fn code(self, error: impl Into<Value>) -> Self {
        self.ctx(Key::Code, error)
    }

    #[inline(always)]
    pub fn id(self, error: impl Into<Value>) -> Self {
        self.ctx(Key::Id, error)
    }

    #[inline(always)]
    pub fn reason(self, error: impl Display) -> Self {
        self.ctx(Key::Reason, error.to_string())
    }

    #[inline(always)]
    pub fn protocol(self, protocol: Protocol) -> Self {
        self.ctx(Key::Protocol, protocol)
    }

    #[inline(always)]
    pub fn document_id(self, id: u32) -> Self {
        self.ctx(Key::DocumentId, id)
    }

    #[inline(always)]
    pub fn account_id(self, id: u32) -> Self {
        self.ctx(Key::AccountId, id)
    }

    #[inline(always)]
    pub fn collection(self, id: impl Into<u8>) -> Self {
        self.ctx(Key::Collection, id.into() as u64)
    }

    #[inline(always)]
    pub fn property(self, id: impl Into<u8>) -> Self {
        self.ctx(Key::Property, id.into() as u64)
    }

    pub fn corrupted_key(key: &[u8], value: Option<&[u8]>, caused_by: &'static str) -> Error {
        Cause::DataCorruption
            .ctx(Key::Key, key)
            .ctx_opt(Key::Value, value)
            .ctx(Key::CausedBy, caused_by)
    }
}

impl Cause {
    #[inline(always)]
    pub fn ctx(self, key: Key, value: impl Into<Value>) -> Error {
        Error::new(self).ctx(key, value)
    }

    #[inline(always)]
    pub fn caused_by(self, error: impl Into<Value>) -> Error {
        Error::new(self).caused_by(error)
    }

    #[inline(always)]
    pub fn reason(self, error: impl Display) -> Error {
        Error::new(self).reason(error)
    }

    #[inline(always)]
    pub fn into_err(self) -> Error {
        Error::new(self)
    }
}

impl Error {
    #[inline(always)]
    pub fn wrap(self, cause: Cause) -> Self {
        Error::new(cause).caused_by(self)
    }
}

impl Value {
    pub fn to_uint(&self) -> Option<u64> {
        match self {
            Self::UInt(value) => Some(*value),
            Self::Int(value) => Some(*value as u64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.as_str()),
            Self::Static(value) => Some(value),
            _ => None,
        }
    }

    pub fn into_string(self) -> Option<Cow<'static, str>> {
        match self {
            Self::String(value) => Some(Cow::Owned(value)),
            Self::Static(value) => Some(Cow::Borrowed(value)),
            _ => None,
        }
    }
}

impl<T> AddContext<T> for Result<T> {
    #[inline(always)]
    fn caused_by(self, location: &'static str) -> Result<T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => Err(err.ctx(Key::CausedBy, location)),
        }
    }

    #[inline(always)]
    fn add_context<F>(self, f: F) -> Result<T>
    where
        F: FnOnce(Error) -> Error,
    {
        match self {
            Ok(value) => Ok(value),
            Err(err) => Err(f(err)),
        }
    }
}

impl<T: std::fmt::Debug, const N: usize> Display for Context<T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.inner)?;
        for (key, value) in self.keys.iter().take(self.keys_size) {
            write!(f, "\n  {:?} = {:?}", key, value)?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}
