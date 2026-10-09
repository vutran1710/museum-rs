//! Where registry bytes live and how access to them is obtained. Implemented per store type
//! (GitHub, S3, …); only `Museum` calls it.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use futures_util::Stream;
use semver::Version;

pub type ByteStream<E> = Pin<Box<dyn Stream<Item = Result<Bytes, E>> + Send>>;

#[derive(Clone, Copy, Debug)]
pub enum Object<'a> {
    Index {
        file_name: &'a str,
    },
    Signature {
        file_name: &'a str,
    },
    Artifact {
        package: &'a str,
        version: &'a Version,
        target: &'a str,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    pub group: String,
    pub file: String,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.group, self.file)
    }
}

pub trait StoreError: std::error::Error + Send + Sync + 'static {
    fn unauthorized(&self) -> bool;
    fn not_found(&self) -> bool;
}

pub trait Store: Send + Sync + 'static {
    type Session: Send + Sync;
    type Error: StoreError;

    fn bootstrap(&self) -> impl Future<Output = Result<Self::Session, Self::Error>> + Send;
    fn open(
        &self,
        session: &Self::Session,
        object: Object<'_>,
    ) -> impl Future<Output = Result<ByteStream<Self::Error>, Self::Error>> + Send;
    fn location(&self, object: Object<'_>) -> Location;
}

/// The write half a store offers for publishing; reading never needs it.
pub trait StoreWriter: Store {
    /// Creates the group if missing and replaces an existing file of the same name.
    fn upload(
        &self,
        session: &Self::Session,
        location: &Location,
        bytes: Vec<u8>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
