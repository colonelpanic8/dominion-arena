//! Complete outbound command layouts. Encoding alone has no network side effects.
use super::{Preference, PreferenceValue, TableRule};
use crate::{
    Error, Result,
    wire::{MAX_COLLECTION, MAX_SAFE_LONG, MAX_STRING_BYTES, Writer},
};
trait Field {
    fn write(&self, w: &mut Writer) -> Result<()>;
}
impl Field for i32 {
    fn write(&self, w: &mut Writer) -> Result<()> {
        w.int(*self);
        Ok(())
    }
}
impl Field for i64 {
    fn write(&self, w: &mut Writer) -> Result<()> {
        if !(-MAX_SAFE_LONG..=MAX_SAFE_LONG).contains(self) {
            return Err(Error::Rejected("unsafe long".into()));
        }
        w.long(*self);
        Ok(())
    }
}
impl Field for f64 {
    fn write(&self, w: &mut Writer) -> Result<()> {
        if !self.is_finite() {
            return Err(Error::Rejected("non-finite double".into()));
        }
        w.double(*self);
        Ok(())
    }
}
impl Field for bool {
    fn write(&self, w: &mut Writer) -> Result<()> {
        w.boolean(*self);
        Ok(())
    }
}
impl Field for String {
    fn write(&self, w: &mut Writer) -> Result<()> {
        if self.len() > MAX_STRING_BYTES {
            return Err(Error::Rejected("string too long".into()));
        }
        w.string(self);
        Ok(())
    }
}
impl<T: Field> Field for Vec<T> {
    fn write(&self, w: &mut Writer) -> Result<()> {
        if self.len() > MAX_COLLECTION {
            return Err(Error::Rejected("collection too long".into()));
        }
        w.int(self.len() as i32);
        for v in self {
            v.write(w)?;
        }
        Ok(())
    }
}
impl<T: Field> Field for Option<T> {
    fn write(&self, w: &mut Writer) -> Result<()> {
        w.boolean(self.is_some());
        if let Some(v) = self {
            v.write(w)?;
        }
        Ok(())
    }
}
impl Field for TableRule {
    fn write(&self, w: &mut Writer) -> Result<()> {
        TableRule::write(self, w)
    }
}
pub struct TextPair(pub String, pub String);
impl Field for TextPair {
    fn write(&self, w: &mut Writer) -> Result<()> {
        self.0.write(w)?;
        self.1.write(w)
    }
}
pub struct TournamentMarker;
impl Field for TournamentMarker {
    fn write(&self, w: &mut Writer) -> Result<()> {
        w.int(-1415139397);
        Ok(())
    }
}
pub struct MatchingCriteria {
    pub bot: i32,
    pub human: bool,
    pub max_rating: i32,
    pub min_rating: i32,
    pub preferences: Vec<(i32, i32)>,
    pub timers: Vec<(i32, i32)>,
}
impl Field for (i32, i32) {
    fn write(&self, w: &mut Writer) -> Result<()> {
        self.0.write(w)?;
        self.1.write(w)
    }
}
impl Field for MatchingCriteria {
    fn write(&self, w: &mut Writer) -> Result<()> {
        self.bot.write(w)?;
        self.human.write(w)?;
        self.max_rating.write(w)?;
        self.min_rating.write(w)?;
        self.preferences.write(w)?;
        self.timers.write(w)
    }
}
impl Field for Preference {
    fn write(&self, w: &mut Writer) -> Result<()> {
        self.id.write(w)?;
        match &self.value {
            PreferenceValue::Text(v) => {
                w.int(0);
                v.write(w)?;
            }
            PreferenceValue::Int(v) => {
                w.int(1);
                v.write(w)?;
            }
            PreferenceValue::Ownership { .. } => {
                w.int(2);
            }
            PreferenceValue::Cards { list_id, cards } => {
                w.int(3);
                list_id.write(w)?;
                cards.write(w)?;
            }
            PreferenceValue::Boolean(v) => {
                w.int(4);
                v.write(w)?;
            }
            PreferenceValue::Matching {
                bot,
                human,
                max_rating,
                min_rating,
                preferences,
                timers,
            } => {
                w.int(5);
                bot.write(w)?;
                human.write(w)?;
                max_rating.write(w)?;
                min_rating.write(w)?;
                preferences.write(w)?;
                timers.write(w)?;
            }
            PreferenceValue::Autoplay(v) => {
                w.int(6);
                v.write(w)?;
            }
            PreferenceValue::Ints(v) => {
                w.int(7);
                v.write(w)?;
            }
            PreferenceValue::Deleted => {
                return Err(Error::Rejected(
                    "upstream has no serializer for deleted preferences".into(),
                ));
            }
        }
        Ok(())
    }
}
macro_rules! commands {
 ($($id:literal => $name:ident { $($field:ident: $ty:ty),* }),* $(,)?) => {
  pub enum Command { $($name {$($field:$ty),*}),* }
  impl Command {
   pub fn opcode(&self)->i32 {match self {$(Self::$name {..}=>$id),*}}
   pub fn encode(&self)->Result<Vec<u8>> {let mut w=Writer::new(); w.int(self.opcode());match self {$(Self::$name {$($field),*}=>{$($field.write(&mut w)?;)*}),*};Ok(w.finish())}
  }
 }
}
commands! {
 0 => SendChat {receiver:String,sender:String,text:String},
 1 => Login {username:String,password:String,version:String,language:i32},
 2 => JoinTable {table:i64,spectator:bool},
 3 => SendInvite {table:i64,player:i32},
 4 => ChangeTableRule {table:i64,rule:TableRule},
 5 => ChangeHost {table:i64,player:i32},
 6 => LeaveTable {table:i64,player:i32},
 7 => SitDown {table:i64},
 8 => StandUp {table:i64,player:i32},
 9 => NewTableRequest {rules:Vec<TableRule>},
 10 => ReadyToStart {table:i64,ready:bool},
 11 => RequestUpdate {kind:i32},
 12 => ChangeUserProperty {preference:Preference},
 13 => LoginWithCode {code:String,version:String},
 14 => LoginWithNewPassword {username:String,version:String,password:String,language:i32},
 15 => ForgotPassword {username:String,version:String},
 16 => ChangePassword {old:String,new:String},
 17 => RequestPurchasePrice {package:i32,end_date:i64},
 18 => Purchase {package:i32,end_date:i64,purchase_time:i64,amount_in_dollar_cents:i32},
 19 => CreateAccount {username:String,password:String,email:String,gender:i32,version:String,language:i32,bonus_code:String},
 20 => RemoveSessions {username:String,password:String,version:String,resign:bool},
 21 => ChangeFriendStatus {player:i32,name:String,add:bool,mutual:bool},
 22 => RequestAutomatch {criteria:MatchingCriteria},
 23 => AddBot {table:i64},
 24 => LoginWithSession {player:i32,version:String,session:String,language:i32},
 25 => ChangeMailStatus {mail:i64,status:i32},
 26 => RequestMailMessages {ids:Vec<i64>},
 27 => RequestMailSummaries {after:i64},
 28 => RequestLeaderboard {from:i32,refresh:bool},
 29 => TerminateGame {},
 30 => ChangeBlacklistStatus {player:i32,name:String,add:bool,mutual:bool},
 31 => ContinueWithBots {table:i64},
 32 => Logout {},
 33 => ValidateBonusCode {player:i32,code:String},
 34 => UseBonusCode {player:i32,code:String,expansion:i32},
 35 => ReconnectGame {},
 36 => RemoveActiveSessions {},
 37 => AnswerQuestion {question:i32,values:Vec<i32>,autoplay:bool},
 38 => Resign {},
 39 => UndoRequest {decision:i32},
 40 => TimeoutRequest {decision:i32,player:i32},
 41 => UndoRequestGranted {player:i32,decision:i32},
 42 => UndoRequestDenied {player:i32,decision:i32},
 43 => UndoRequestCancelled {player:i32,decision:i32},
 44 => Ping {},
 45 => RequestServerState {version:String,player:Option<i32>,session:Option<String>},
 46 => ReconnectSession {player:i32,version:String,session:String},
 47 => RequestConnectToOtherServer {},
 48 => PreviewRemovalSelection {question:i32,indices:Vec<i32>},
 49 => EndTurn {},
 50 => SubmitFeedback {form:String,fields:Vec<TextPair>},
 51 => AnimationCompensationAck {},
 52 => TournamentMatchCommand {table:i64,action:i32,match_id:i32,marker:TournamentMarker,kind:i32},
 53 => PersistUserProperty {preference:Preference},
 54 => RequestCardStats {min_level:f64,player:String,config:String},
 55 => RequestCardPerTurn {min_level:f64,player:String,card:i32},
 56 => RequestScorigami {},
 57 => RequestPlayerProfile {player:i32,rating:i32,period:i32,offset:i32},
 58 => UpdatePlayerProfile {text:String,country:String,rating:i32,avatar:String},
 59 => RequestHeadToHead {player:i32,opponent:String,rating:i32},
 60 => RequestPlayerAvatars {players:Vec<i32>},
 61 => RequestPlayerResults {player:i32,rating:i32,count:i32},
 62 => RequestPlayerSearch {name:String},
 63 => RequestStatsViews {},
 64 => SaveStatsView {name:String,config:String},
 65 => DeleteStatsView {name:String},
 66 => RequestStatsProfiles {config:String},
 67 => RequestStatsComparison {left_player:String,left_level:f64,right_player:String,right_level:f64}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_command_matches_the_upstream_serializer() {
        let commands = vec![
            Command::SendChat {
                receiver: "fixture".into(),
                sender: "fixture".into(),
                text: "fixture".into(),
            },
            Command::Login {
                username: "fixture".into(),
                password: "fixture".into(),
                version: "2.3.7".into(),
                language: -1,
            },
            Command::JoinTable {
                table: 100,
                spectator: true,
            },
            Command::SendInvite {
                table: 100,
                player: 0,
            },
            Command::ChangeTableRule {
                table: 100,
                rule: TableRule {
                    id: 19,
                    value: super::super::RuleValue::Boolean(true),
                },
            },
            Command::ChangeHost {
                table: 100,
                player: 0,
            },
            Command::LeaveTable {
                table: 100,
                player: 0,
            },
            Command::SitDown { table: 100 },
            Command::StandUp {
                table: 100,
                player: 0,
            },
            Command::NewTableRequest { rules: vec![] },
            Command::ReadyToStart {
                table: 100,
                ready: true,
            },
            Command::RequestUpdate { kind: 0 },
            Command::ChangeUserProperty {
                preference: Preference {
                    id: 6,
                    value: PreferenceValue::Boolean(true),
                },
            },
            Command::LoginWithCode {
                code: "fixture".into(),
                version: "2.3.7".into(),
            },
            Command::LoginWithNewPassword {
                username: "fixture".into(),
                version: "2.3.7".into(),
                password: "fixture".into(),
                language: -1,
            },
            Command::ForgotPassword {
                username: "fixture".into(),
                version: "2.3.7".into(),
            },
            Command::ChangePassword {
                old: "fixture".into(),
                new: "fixture".into(),
            },
            Command::RequestPurchasePrice {
                package: 0,
                end_date: 100,
            },
            Command::Purchase {
                package: 0,
                end_date: 100,
                purchase_time: 100,
                amount_in_dollar_cents: 0,
            },
            Command::CreateAccount {
                username: "fixture".into(),
                password: "fixture".into(),
                email: "fixture".into(),
                gender: 0,
                version: "2.3.7".into(),
                language: -1,
                bonus_code: "fixture".into(),
            },
            Command::RemoveSessions {
                username: "fixture".into(),
                password: "fixture".into(),
                version: "2.3.7".into(),
                resign: true,
            },
            Command::ChangeFriendStatus {
                player: 0,
                name: "fixture".into(),
                add: true,
                mutual: true,
            },
            Command::RequestAutomatch {
                criteria: MatchingCriteria {
                    bot: 0,
                    human: true,
                    max_rating: 0,
                    min_rating: 0,
                    preferences: vec![],
                    timers: vec![],
                },
            },
            Command::AddBot { table: 100 },
            Command::LoginWithSession {
                player: 0,
                version: "2.3.7".into(),
                session: "fixture".into(),
                language: -1,
            },
            Command::ChangeMailStatus {
                mail: 100,
                status: 0,
            },
            Command::RequestMailMessages { ids: vec![100] },
            Command::RequestMailSummaries { after: 100 },
            Command::RequestLeaderboard {
                from: 0,
                refresh: true,
            },
            Command::TerminateGame {},
            Command::ChangeBlacklistStatus {
                player: 0,
                name: "fixture".into(),
                add: true,
                mutual: true,
            },
            Command::ContinueWithBots { table: 100 },
            Command::Logout {},
            Command::ValidateBonusCode {
                player: 0,
                code: "fixture".into(),
            },
            Command::UseBonusCode {
                player: 0,
                code: "fixture".into(),
                expansion: 0,
            },
            Command::ReconnectGame {},
            Command::RemoveActiveSessions {},
            Command::AnswerQuestion {
                question: 0,
                values: vec![0],
                autoplay: false,
            },
            Command::Resign {},
            Command::UndoRequest { decision: 0 },
            Command::TimeoutRequest {
                decision: 0,
                player: 0,
            },
            Command::UndoRequestGranted {
                player: 0,
                decision: 0,
            },
            Command::UndoRequestDenied {
                player: 0,
                decision: 0,
            },
            Command::UndoRequestCancelled {
                player: 0,
                decision: 0,
            },
            Command::Ping {},
            Command::RequestServerState {
                version: "2.3.7".into(),
                player: Some(0),
                session: Some("fixture".into()),
            },
            Command::ReconnectSession {
                player: 0,
                version: "2.3.7".into(),
                session: "fixture".into(),
            },
            Command::RequestConnectToOtherServer {},
            Command::PreviewRemovalSelection {
                question: 0,
                indices: vec![0],
            },
            Command::EndTurn {},
            Command::SubmitFeedback {
                form: "fixture".into(),
                fields: vec![TextPair("client_language".into(), "English".into())],
            },
            Command::AnimationCompensationAck {},
            Command::TournamentMatchCommand {
                table: 100,
                action: 0,
                match_id: 0,
                marker: TournamentMarker,
                kind: 0,
            },
            Command::PersistUserProperty {
                preference: Preference {
                    id: 6,
                    value: PreferenceValue::Boolean(true),
                },
            },
            Command::RequestCardStats {
                min_level: 1.25,
                player: "fixture".into(),
                config: "fixture".into(),
            },
            Command::RequestCardPerTurn {
                min_level: 1.25,
                player: "fixture".into(),
                card: 0,
            },
            Command::RequestScorigami {},
            Command::RequestPlayerProfile {
                player: 0,
                rating: 0,
                period: 0,
                offset: 0,
            },
            Command::UpdatePlayerProfile {
                text: "fixture".into(),
                country: "fixture".into(),
                rating: 0,
                avatar: "fixture".into(),
            },
            Command::RequestHeadToHead {
                player: 0,
                opponent: "fixture".into(),
                rating: 0,
            },
            Command::RequestPlayerAvatars { players: vec![0] },
            Command::RequestPlayerResults {
                player: 0,
                rating: 0,
                count: 0,
            },
            Command::RequestPlayerSearch {
                name: "fixture".into(),
            },
            Command::RequestStatsViews {},
            Command::SaveStatsView {
                name: "fixture".into(),
                config: "fixture".into(),
            },
            Command::DeleteStatsView {
                name: "fixture".into(),
            },
            Command::RequestStatsProfiles {
                config: "fixture".into(),
            },
            Command::RequestStatsComparison {
                left_player: "fixture".into(),
                left_level: 1.25,
                right_player: "fixture".into(),
                right_level: 1.25,
            },
        ];
        let fixtures: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../tests/fixtures/outgoing.json")).unwrap();
        assert_eq!(commands.len(), 68);
        for (command, fixture) in commands.iter().zip(fixtures) {
            let bytes = command.encode().unwrap();
            let hex = bytes.iter().map(|v| format!("{v:02x}")).collect::<String>();
            assert_eq!(command.opcode() as i64, fixture["opcode"].as_i64().unwrap());
            assert_eq!(
                hex,
                fixture["hex"].as_str().unwrap(),
                "opcode {}",
                command.opcode()
            );
        }
    }
}
