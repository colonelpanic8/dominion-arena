//! Chat: the lobby's, each table's and each game's, the last messages of each kept in
//! memory.

use super::{Arena, Chat, Error, Ident, Result, State, bad, limited};
use crate::hub::{lock, now_ms};
use dominion_protocol::lobby::{ChatKind, ChatLog, ChatMessage};
use std::collections::VecDeque;

const KEPT: usize = 200;
const MAX_CHARS: usize = 500;

impl State {
    pub(super) fn say(&mut self, channel: &str, from: &str, kind: ChatKind, text: &str) {
        let chat = self.chats.entry(channel.to_string()).or_insert(Chat {
            next: 1,
            messages: VecDeque::new(),
        });
        chat.messages.push_back(ChatMessage {
            seq: chat.next,
            at: now_ms(),
            from: from.to_string(),
            kind,
            text: text.to_string(),
        });
        chat.next += 1;
        if chat.messages.len() > KEPT {
            chat.messages.pop_front();
        }
    }
}

/// One line of what was typed, or why it can't be said.
fn line(text: &str) -> Result<String> {
    let text: String = (text.trim().chars())
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control() && !matches!(c, '\u{2028}' | '\u{2029}'))
        .collect();
    match text.chars().count() {
        0 => bad("say something"),
        n if n > MAX_CHARS => bad(format!("a message has at most {MAX_CHARS} characters")),
        _ => Ok(text),
    }
}

impl Arena {
    /// A channel's messages after number `after`.
    pub fn chat_log(&self, channel: &str, after: u64) -> ChatLog {
        let state = lock(&self.state);
        let messages = state.chats.get(channel).map(|c| {
            let new = c.messages.iter().filter(|m| m.seq > after);
            new.cloned().collect()
        });
        ChatLog {
            messages: messages.unwrap_or_default(),
        }
    }

    /// `ident` says `text` in the lobby (`lobby`) or at the table they sit at
    /// (`table-<id>`).
    pub fn say(&self, ident: &Ident, channel: &str, text: &str) -> Result<()> {
        let text = line(text)?;
        let name = self.user(&ident.user)?.name;
        let mut state = lock(&self.state);
        let seated = state.seated.get(&ident.user);
        let allowed = channel == "lobby"
            || channel
                .strip_prefix("table-")
                .is_some_and(|t| seated.is_some_and(|s| s == t));
        if !allowed {
            return Err(Error::Forbidden("you can't speak there".into()));
        }
        limited(&self.limits.chat, ident.user.clone(), "messages")?;
        state.touch(&ident.user);
        state.say(channel, &name, ChatKind::Player, &text);
        drop(state);
        self.chat.send_modify(|v| *v += 1);
        Ok(())
    }

    /// `name` says `text` in game `game`'s chat. `speaker` is who to count the message
    /// against.
    pub fn say_in_game(
        &self,
        game: &str,
        speaker: &str,
        name: &str,
        kind: ChatKind,
        text: &str,
    ) -> Result<()> {
        let text = line(text)?;
        limited(&self.limits.chat, speaker.to_string(), "messages")?;
        lock(&self.state).say(&format!("game-{game}"), name, kind, &text);
        self.hub.chat_said(game);
        Ok(())
    }

    /// The name `ident` speaks under.
    pub fn name_of(&self, ident: &Ident) -> Result<String> {
        Ok(self.user(&ident.user)?.name)
    }
}
