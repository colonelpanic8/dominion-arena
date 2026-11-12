//! Who this browser is signed in as. The session itself is a cookie the server sets;
//! here is only what it says about its account.

use crate::api::{self, ApiError};
use dioxus::prelude::*;
use dominion_protocol::lobby::Me;

/// The account signed in, once known.
#[derive(Clone, Copy)]
pub struct Account {
    pub me: Signal<Option<Me>>,
    /// Whether the server has been asked yet: until then `me` being `None` says nothing.
    pub loaded: Signal<bool>,
}

/// Provides [`Account`] to the app and asks the server who is signed in.
pub fn use_provide_account() -> Account {
    let account = use_context_provider(|| Account {
        me: Signal::new(None),
        loaded: Signal::new(false),
    });
    use_future(move || async move {
        account.refresh().await;
    });
    account
}

pub fn use_account() -> Account {
    use_context::<Account>()
}

impl Account {
    /// The signed-in account's name.
    pub fn name(&self) -> Option<String> {
        self.me.read().as_ref().map(|me| me.user.name.clone())
    }

    fn set(mut self, me: Option<Me>) {
        if *self.me.peek() != me {
            self.me.set(me);
        }
        if !*self.loaded.peek() {
            self.loaded.set(true);
        }
    }

    fn took(self, result: Result<Me, ApiError>) -> Result<Me, ApiError> {
        if let Ok(me) = &result {
            self.set(Some(me.clone()));
        }
        result
    }

    /// Asks the server who is signed in.
    pub async fn refresh(self) {
        match api::me().await {
            Ok(me) => self.set(Some(me)),
            Err(e) if e.is_unauthorized() => self.set(None),
            // Unreachable: what is shown stays.
            Err(_) => {}
        }
    }

    pub async fn login(self, name: &str, password: &str) -> Result<Me, ApiError> {
        self.took(api::login(name, password).await)
    }

    /// Registers an account; signed in as a guest, the guest becomes it.
    pub async fn register(self, name: &str, password: &str) -> Result<Me, ApiError> {
        self.took(api::register(name, password).await)
    }

    pub async fn guest(self, name: Option<&str>) -> Result<Me, ApiError> {
        self.took(api::guest(name).await)
    }

    pub async fn logout(self) {
        if api::logout().await.is_ok() {
            self.set(None);
        }
    }

    /// The account signed in, signing in as a guest if nobody is: for what needs someone
    /// to be (sitting down, searching, speaking) on a server that takes guests.
    pub async fn ensure(self) -> Result<Me, ApiError> {
        if !*self.loaded.peek() {
            self.refresh().await;
        }
        let me = self.me.peek().clone();
        match me {
            Some(me) => Ok(me),
            None => self.guest(None).await,
        }
    }
}
