use paydirt_store::{Fault, Frame, Result, TopicGrants, notify::{Policy,Rows}};
use serde_json::Value;
pub struct File(pub std::path::PathBuf);
impl Policy for File {
    fn resolve(&self,_:&mut Rows<'_,'_>,frame:&Frame,_:&[u8],_:&str,_:&Value)->Result<(Vec<u8>,TopicGrants)> {
        let mut grants=frame.topics.clone();
        if std::fs::read_to_string(&self.0).map_err(|e|Fault::unavailable(e.to_string()))?.trim()=="revoked" {
            grants.read.retain(|t|t!="n:jobs");
        }
        Ok((frame.actor.clone().ok_or_else(||Fault::denied("reader required"))?,grants))
    }
}
