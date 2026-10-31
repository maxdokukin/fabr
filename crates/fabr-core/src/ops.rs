use crate::dto::{
    Empty, GetTreeReq, HostReq, InitChildrenReq, InstallKeyReq, ListDirsReq, NodeRef, OpenUrlReq, RemoveNodeReq, RevealPathReq,
    SaveMetaReq, SaveSourcesReq, SourceIdReq, SourcePasswordReq, Stamp,
};
use crate::error::FabrError;
use crate::fabric::{self, FabricCreateReq, FabricListReq, FabricReadReq, FabricStampReq, FabricWriteReq, ImportTodosReq};
use crate::settings::Settings;
use crate::setup::{self, CheckReq, Ctx, LinkReq, SecretsWriteReq, SettingsWriteReq};
use crate::sources::Source;
use crate::workspace::{RemoteCreateReq, RemoteSourceReq, RemoteWriteReq, Workspace};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Get,
    Post,
}

#[derive(Debug, Clone, Copy)]
pub struct Op {
    pub name: &'static str,
    pub verb: Verb,
    pub http: bool,
}

#[derive(Debug)]
pub enum Desktop {
    OpenPath(NodeRef),
    Reveal(RevealPathReq),
    OpenUrl(OpenUrlReq),
    OpenSession(OpenSessionReq),
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenSessionReq {
    pub folder: String,
    pub session_id: String,
}

#[derive(Debug)]
pub enum Done {
    Json(Value),
    Desktop(Desktop),
}

fn json<T: Serialize>(v: T) -> Result<Done, FabrError> {
    serde_json::to_value(v).map(Done::Json).map_err(|e| FabrError::Io(e.to_string()))
}

macro_rules! ops {
    ($($name:ident $verb:ident $http:literal ($req:ty) |$ws:pat_param, $r:pat_param| $body:expr;)*) => {
        pub const OPS: &[Op] = &[$(Op { name: stringify!($name), verb: Verb::$verb, http: $http }),*];

        pub fn dispatch(ws: &Workspace, name: &str, body: Value) -> Result<Done, FabrError> {
            match name {
                $(stringify!($name) => {
                    let $r: $req = serde_json::from_value(body).map_err(|e| FabrError::BadRequest(format!("bad request body: {e}")))?;
                    let $ws = ws;
                    $body
                })*
                _ => Err(FabrError::NotFound(format!("no such operation {name:?}"))),
            }
        }
    };
}

ops! {
    get_tree Get true (GetTreeReq) |ws, r| json(ws.get_tree_req(r));
    get_stamp Get true (Empty) |ws, _| json(ws.get_stamp());
    list_sources Get true (Empty) |ws, _| json(ws.list_sources());
    get_settings Get true (Empty) |ws, _| json(ws.get_settings());
    save_meta Post true (SaveMetaReq) |ws, r| json(ws.save_meta(r)?);
    init_node Post true (NodeRef) |ws, r| json(ws.init_node(r)?);
    remove_node Post true (RemoveNodeReq) |ws, r| json(ws.remove_node(r)?);
    list_dirs Post true (ListDirsReq) |ws, r| json(ws.list_dirs(r)?);
    init_children Post true (InitChildrenReq) |ws, r| json(ws.init_children(r)?);
    save_sources Post true (SaveSourcesReq) |ws, r| json(ws.save_sources(r.sources)?);
    test_source Post true (Source) |ws, r| json(ws.test_source(r));
    save_settings Post true (Settings) |ws, r| json(ws.save_settings(r)?);
    host_key Post true (HostReq) |ws, r| json(ws.host_key(r)?);
    trust_host Post true (HostReq) |ws, r| json(ws.trust_host(r)?);
    set_source_password Post true (SourcePasswordReq) |ws, r| json(ws.set_source_password(r)?);
    has_source_password Post true (SourceIdReq) |ws, r| json(ws.has_source_password(r));
    install_key Post true (InstallKeyReq) |ws, r| json(ws.install_key(r)?);
    fabric_list Post true (FabricListReq) |_, r| json(fabric::list(&r)?);
    fabric_read Post true (FabricReadReq) |_, r| json(fabric::read(&r)?);
    fabric_write Post true (FabricWriteReq) |_, r| json(fabric::write(&r)?);
    fabric_create Post true (FabricCreateReq) |_, r| json(fabric::create(&r)?);
    fabric_remote_list Post true (RemoteSourceReq) |ws, r| json(ws.remote_tasks(&r)?);
    fabric_remote_stamp Post true (RemoteSourceReq) |ws, r| json(ws.remote_task_stamp(&r)?);
    fabric_remote_write Post true (RemoteWriteReq) |ws, r| json(ws.remote_task_write(&r)?);
    fabric_remote_create Post true (RemoteCreateReq) |ws, r| json(ws.remote_task_create(&r)?);
    import_todos Post true (ImportTodosReq) |_, r| json(fabric::import_todos(&r)?);
    fabric_stamp Post true (FabricStampReq) |_, r| json(Stamp { stamp: fabric::stamp(&r)? });
    fabric_settings_read Post true (Empty) |_, _| json(setup::settings_read(&Ctx::from_env())?);
    fabric_home Post true (Empty) |_, _| json(Ctx::from_env().home.to_string_lossy().into_owned());
    fabric_settings_write Post true (SettingsWriteReq) |_, r| json(setup::settings_write(&Ctx::from_env(), r)?);
    fabric_secrets_write Post true (SecretsWriteReq) |_, r| json(setup::secrets_write(&Ctx::from_env(), r)?);
    fabric_secrets_status Post true (Empty) |_, _| json(setup::secrets_status(&Ctx::from_env()));
    fabric_check Post true (CheckReq) |_, r| json(setup::check(&Ctx::from_env(), r)?);
    fabric_telegram_link Post true (LinkReq) |_, r| json(setup::link(&Ctx::from_env(), r));
    open_path Post true (NodeRef) |_, r| Ok(Done::Desktop(Desktop::OpenPath(r)));
    reveal_path Post true (RevealPathReq) |_, r| Ok(Done::Desktop(Desktop::Reveal(r)));
    open_url Post false (OpenUrlReq) |_, r| Ok(Done::Desktop(Desktop::OpenUrl(r)));
    fabric_open_session Post false (OpenSessionReq) |_, r| Ok(Done::Desktop(Desktop::OpenSession(r)));
}

pub fn url_allowed(u: &str) -> bool {
    let l = u.to_ascii_lowercase();
    (l.starts_with("http://") || l.starts_with("https://")) && !u.chars().any(|c| c.is_whitespace() || c.is_control())
}

pub fn quote_sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn session_command(req: &OpenSessionReq) -> Result<String, FabrError> {
    let id = req.session_id.as_str();
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(FabrError::BadRequest("that is not a session id".into()));
    }
    let dir = std::path::Path::new(&req.folder);
    if !dir.is_absolute() || !dir.is_dir() {
        return Err(FabrError::BadRequest(format!("{} is not a folder on this computer", req.folder)));
    }
    Ok(format!("cd {} && claude --resume {id}", quote_sh(&req.folder)))
}

#[cfg(test)]
mod session_tests {
    use super::*;

    fn req(folder: &str, id: &str) -> OpenSessionReq {
        OpenSessionReq { folder: folder.into(), session_id: id.into() }
    }

    #[test]
    fn command_quotes_the_folder_and_checks_the_id() {
        let d = tempfile::tempdir().unwrap();
        let odd = d.path().join("it's here");
        std::fs::create_dir(&odd).unwrap();
        let cmd = session_command(&req(odd.to_str().unwrap(), "3f2b8c1e-4d5a")).unwrap();
        assert_eq!(cmd, format!("cd '{}' && claude --resume 3f2b8c1e-4d5a", odd.to_str().unwrap().replace('\'', "'\\''")));
        for bad in ["", "a b", "x;rm -rf ~", "$(id)", "a'b"] {
            assert!(session_command(&req(d.path().to_str().unwrap(), bad)).is_err(), "{bad}");
        }
        assert!(session_command(&req("relative", "abc")).is_err());
        assert!(session_command(&req(d.path().join("nope").to_str().unwrap(), "abc")).is_err());
    }
}

#[cfg(test)]
mod url_tests {
    use super::url_allowed;

    #[test]
    fn only_http_and_https() {
        assert!(url_allowed("https://login.tailscale.com/a/abc"));
        assert!(url_allowed("http://localhost:1/x"));
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "ftp://x", "https://a b", "", "tailscale://x"] {
            assert!(!url_allowed(bad), "{bad}");
        }
    }
}
