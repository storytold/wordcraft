//! Open and Save against cloud storage, driven through the real interface with a service that
//! lives inside the test.
//!
//! The service answers on the calling thread, so a job posted during a frame is in the inbox
//! before the next one. Everything else is the shipping code: the Backstage pages, `WordApp::run`,
//! the Save Changes prompt, the conflict modal and the docx path.

use std::sync::{Arc, Mutex};

use egui_kittest::kittest::Queryable as _;
use serde_json::{Value as Json, json};
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::Session;
use wordcraft_ui_egui::cloud::{self, Account, Capability, CloudBackend, Entry, EntryKind, Event, Fault, Job, Post};
use wordcraft_ui_egui::{Services, WordApp};

type Harness = egui_kittest::Harness<'static, WordApp>;

// ---------------------------------------------------------------- a service in memory

#[derive(Default)]
struct State {
    /// Every file and folder, by its own path. A folder is listed in its parent like any other
    /// entry.
    files: Vec<(Entry, Vec<u8>)>,
    /// Incoming shares, which have no folder of this user's to sit in.
    shared: Vec<(Entry, Vec<u8>)>,
    capabilities: Vec<Capability>,
    linked: bool,
    /// What the service was asked to do, so a test can prove a save went out once and was not
    /// retried.
    writes: Vec<(String, Option<String>, usize)>,
    creates: Vec<(String, String, usize)>,
    reads: Vec<String>,
}

impl State {
    fn account(&self) -> Result<Account, Fault> {
        if !self.linked {
            return Err(Fault::NotLinked);
        }
        Ok(Account { display: "Ada Lovelace".to_owned(), capabilities: self.capabilities.clone(), provider_linked: true })
    }

    fn find(&self, id: &str) -> Option<&(Entry, Vec<u8>)> {
        self.files.iter().chain(self.shared.iter()).find(|(e, _)| e.id == id)
    }

    /// Somebody else saves over a file, which is what makes the next conditional write fail.
    fn overwrite(&mut self, id: &str, bytes: Vec<u8>) {
        if let Some(slot) = self.files.iter_mut().chain(self.shared.iter_mut()).find(|(e, _)| e.id == id) {
            slot.0.etag = Some("theirs".to_owned());
            slot.0.size = Some(bytes.len() as u64);
            slot.1 = bytes;
        }
    }

    fn answer(&mut self, job: Job) -> Vec<Event> {
        // Signing out is the one job that needs no account: the interface clears its own state
        // before the job is ever sent.
        if matches!(job, Job::SignOut) {
            self.linked = false;
            return Vec::new();
        }
        if !self.linked && !matches!(job, Job::LinkStart | Job::LinkPoll(_)) {
            return vec![match job {
                Job::Account => Event::Account(Err(Fault::NotLinked)),
                Job::List(path) => Event::Listed { path, entries: Err(Fault::NotLinked) },
                Job::SharedWithMe => Event::Shared(Err(Fault::NotLinked)),
                Job::Read { entry, into, .. } => Event::Read { entry, into, bytes: Err(Fault::NotLinked) },
                Job::Write { uid, .. } | Job::Create { uid, .. } => Event::Saved { uid, entry: Err(Fault::NotLinked) },
                Job::LinkStart | Job::LinkPoll(_) => Event::Linked(Ok(false)),
                Job::Conversation(_) => Event::Conversation(Err(Fault::NotLinked)),
                // Handled above, before this arm is reached.
                Job::SignOut => return Vec::new(),
            }];
        }
        match job {
            Job::SignOut => Vec::new(),
            Job::Conversation(_) => {
                if !self.capabilities.contains(&Capability::Conversation) {
                    let why = "this provider has nowhere to talk".to_owned();
                    return vec![Event::Conversation(Err(Fault::Refused(why)))];
                }
                let room = craft_cloud::Conversation { url: "https://cloud.example/call/room".to_owned(), name: "Talk".to_owned() };
                vec![Event::Conversation(Ok(room))]
            }
            Job::Account => vec![Event::Account(self.account())],
            Job::LinkStart => {
                vec![Event::Browser { url: "https://cloud.example/link/abc".to_owned(), handle: Some("abc".to_owned()) }]
            }
            Job::LinkPoll(_) => vec![Event::Linked(Ok(self.linked))],
            Job::List(path) => {
                let entries = self.files.iter().filter(|(e, _)| parent(&e.path) == path).map(|(e, _)| e.clone()).collect();
                vec![Event::Listed { path, entries: Ok(entries) }]
            }
            Job::SharedWithMe => {
                if !self.capabilities.contains(&Capability::SharedWithMe) {
                    return vec![Event::Shared(Err(Fault::Refused("this provider has no incoming shares".to_owned())))];
                }
                vec![Event::Shared(Ok(self.shared.iter().map(|(e, _)| e.clone()).collect()))]
            }
            Job::Read { entry, into } => {
                self.reads.push(entry.id.clone());
                let bytes = match self.find(&entry.id) {
                    Some((_, bytes)) => Ok(bytes.clone()),
                    None => Err(Fault::Refused("no such file".to_owned())),
                };
                vec![Event::Read { entry, into, bytes }]
            }
            Job::Write { uid, entry, bytes } => {
                self.writes.push((entry.id.clone(), entry.etag.clone(), bytes.len()));
                let stored = self.find(&entry.id).map(|(e, _)| e.etag.clone());
                let answer = match stored {
                    None => Err(Fault::Refused("no such file".to_owned())),
                    // The condition the save was made under. A stored copy that moved on refuses
                    // it, which is the whole point.
                    Some(stored) if stored != entry.etag => Err(Fault::Conflict),
                    Some(_) => {
                        let slot = self.files.iter_mut().chain(self.shared.iter_mut()).find(|(e, _)| e.id == entry.id);
                        match slot {
                            Some(slot) => {
                                slot.0.etag = Some(format!("{}+", slot.0.etag.clone().unwrap_or_default()));
                                slot.0.size = Some(bytes.len() as u64);
                                slot.1 = bytes;
                                Ok(slot.0.clone())
                            }
                            None => Err(Fault::Refused("no such file".to_owned())),
                        }
                    }
                };
                vec![Event::Saved { uid, entry: answer }]
            }
            Job::Create { uid, folder, name, bytes } => {
                self.creates.push((folder.clone(), name.clone(), bytes.len()));
                let path = format!("{}/{name}", folder.trim_end_matches('/'));
                let entry = file_entry(&path, None);
                self.files.push((entry.clone(), bytes));
                vec![Event::Saved { uid, entry: Ok(entry) }]
            }
        }
    }
}

fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rfind('/') {
        Some(0) | None => String::new(),
        Some(cut) => path[..cut].to_owned(),
    }
}

fn file_entry(path: &str, mime: Option<&str>) -> Entry {
    Entry {
        id: path.to_owned(),
        room: format!("room:{path}"),
        name: path.rsplit('/').next().unwrap_or(path).to_owned(),
        path: path.to_owned(),
        kind: EntryKind::File,
        size: Some(1024),
        modified: Some("2026-10-01T09:00:00Z".to_owned()),
        mime: mime.map(str::to_owned),
        etag: Some("v1".to_owned()),
        writable: true,
        shared_by: None,
    }
}

fn folder_entry(path: &str) -> Entry {
    Entry { kind: EntryKind::Folder, size: None, etag: None, mime: None, ..file_entry(path, None) }
}

/// A document of one paragraph, as real bytes of whatever format `name` says.
fn document_bytes(name: &str, text: &str) -> Vec<u8> {
    wordcraft_engine::io::save_bytes(name, &Document::from_text(text)).unwrap()
}

struct Fake {
    state: Arc<Mutex<State>>,
    opened_urls: Arc<Mutex<Vec<String>>>,
}

/// The worker half, answering on the calling thread. A job posted during a frame is therefore
/// answered before the next one.
struct Service(Arc<Mutex<State>>);

impl CloudBackend for Service {
    fn submit(&self, job: Job, post: Post) {
        for event in self.0.lock().unwrap().answer(job) {
            post.send(event);
        }
    }
}

/// A sharing service that answers out of a fixed set of people.
///
/// The share dialog is a craft-ui control with its own tests. This exists so the window has
/// somewhere to share to. That is what decides whether the title bar draws a split button at all.
struct Sharing;

impl craft_ui::share::ShareBackend for Sharing {
    fn submit(&self, request: craft_ui::share::Request, reply: craft_ui::share::Replier) {
        use craft_cloud::{Contact, Permission, Share, ShareTarget};
        use craft_ui::share::{Answer, Request};
        let person = |id: &str, name: &str, email: &str| Contact {
            id: id.to_owned(),
            target: ShareTarget::User(id.to_owned()),
            name: name.to_owned(),
            email: Some(email.to_owned()),
            avatar: None,
            is_group: false,
        };
        match request {
            Request::Capabilities => reply.answer(Answer::Capabilities {
                capabilities: vec![
                    Capability::Sharing,
                    Capability::Contacts,
                    Capability::LinkShares,
                    Capability::LinkPassword,
                    Capability::ShareNotify,
                ],
                linked: true,
            }),
            Request::Lookup { query, .. } => {
                let everybody = [
                    person("dana", "Dana Okafor", "dana@example.org"),
                    person("priya", "Priya Raman", "priya@example.org"),
                    person("tomas", "Tom\u{e1}s Alvarez", "tomas@example.org"),
                ];
                let want = query.to_lowercase();
                reply.answer(Answer::Lookup(everybody.into_iter().filter(|c| c.name.to_lowercase().contains(&want)).collect()));
            }
            Request::Access { .. } => reply.answer(Answer::Access(vec![
                Share {
                    id: "s1".to_owned(),
                    target: ShareTarget::User("dana".to_owned()),
                    display: Some("Dana Okafor".to_owned()),
                    permission: Permission::Edit,
                    url: None,
                    note: None,
                    created: None,
                },
                Share {
                    id: "s2".to_owned(),
                    target: ShareTarget::Link,
                    display: None,
                    permission: Permission::View,
                    url: Some("https://cloud.example/s/abc123".to_owned()),
                    note: None,
                    created: None,
                },
            ])),
            other => reply.refuse(format!("{other:?} is not offered here")),
        }
    }
}

fn harness(state: State) -> (Harness, Fake) {
    build(state, false)
}

/// The same window, with an offscreen renderer attached so a frame can be written to a file. Only
/// the render test asks for one. Starting wgpu for every behaviour test would cost seconds and
/// prove nothing.
fn rendering(state: State) -> (Harness, Fake) {
    build(state, true)
}

fn build(state: State, rendering: bool) -> (Harness, Fake) {
    let store = Arc::new(Mutex::new(state));
    let urls = Arc::new(Mutex::new(Vec::new()));
    let captured = urls.clone();
    let services = Services { open_url: Some(Box::new(move |url: &str| captured.lock().unwrap().push(url.to_owned()))), ..Default::default() };
    let builder = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 980.0));
    // Two device pixels per point, so a render is legible when somebody looks at it. The behaviour
    // tests keep one, where a click lands on the point it was aimed at.
    let builder = if rendering { builder.with_pixels_per_point(2.0).wgpu() } else { builder };
    let mut h = builder.build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        {
            let mut app = WordApp::new(Session::new(Document::from_text("Draft")), services);
            app.cloud.connect(Service(store.clone()));
            // Only a render wants one. A behaviour test that asked for capabilities it never uses
            // would answer a question nobody put.
            if rendering {
                app.share.connect(Sharing);
            }
            app
        },
    );
    h.run_steps(4);
    (h, Fake { state: store, opened_urls: urls })
}

/// A linked account with two folders of files and one incoming share.
fn furnished() -> State {
    let mut state = State {
        linked: true,
        capabilities: vec![Capability::Files, Capability::SharedWithMe, Capability::Sharing, Capability::Etags],
        ..State::default()
    };
    state.files.push((folder_entry("/Reports"), Vec::new()));
    state.files.push((
        file_entry("/Report.docx", Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")),
        document_bytes("Report.docx", "Forty two"),
    ));
    state.files.push((file_entry("/accounts.pdf", Some("application/pdf")), b"%PDF-1.4".to_vec()));
    // No extension at all, and a content type that says what it is.
    state.files.push((file_entry("/exported", Some("text/markdown")), b"Exported notes\n".to_vec()));
    state.files.push((file_entry("/Reports/Q3.docx", None), document_bytes("Q3.docx", "Seven")));
    let mut team = file_entry("/shared/Team.docx", None);
    team.shared_by = Some("Dana".to_owned());
    state.shared.push((team, document_bytes("Team.docx", "Ninety nine")));
    state
}

fn open_page(h: &mut Harness) {
    h.state_mut().run("ui.backstage", json!({"value": true, "page": "open"})).unwrap();
    h.run_steps(4);
}

fn click(h: &mut Harness, label: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
    h.run_steps(4);
}

fn status(h: &mut Harness) -> Json {
    h.state_mut().run("cloud.status", json!({})).unwrap()
}

/// The document's text, which is what proves the bytes went through the real open path.
fn body(h: &Harness) -> String {
    h.state().session.doc.plain_text(StoryRef::Body)
}

// ---------------------------------------------------------------- the tests

#[test]
fn the_open_page_lists_my_files_and_what_was_shared_with_me() {
    let (mut h, _fake) = harness(furnished());
    open_page(&mut h);
    assert!(h.query_by_label("My files").is_some(), "the Open page browses your own cloud files");
    assert!(h.query_by_label("Shared with me").is_some(), "and what other people shared with you, beside it");
    assert!(h.query_by_label("Report.docx").is_some());
    assert!(h.query_by_label("Reports").is_some(), "a folder is offered so the user can walk into it");
    assert!(h.query_by_label("exported").is_some(), "no extension, and a content type that says Markdown");
    assert!(h.query_by_label("accounts.pdf").is_none(), "WordCraft cannot open a PDF, so it is not offered");
    assert!(h.query_by_label("Team.docx").is_some());
    let shared = status(&mut h)["shared"].clone();
    assert_eq!(shared[0]["name"], "Team.docx");
    assert_eq!(shared[0]["sharedBy"], "Dana", "each incoming share says who shared it");
    // Walking into a folder lists it.
    click(&mut h, "Reports");
    assert_eq!(status(&mut h)["folder"], "/Reports");
    assert!(h.query_by_label("Q3.docx").is_some());
    click(&mut h, "Up");
    assert_eq!(status(&mut h)["folder"], "");
}

#[test]
fn shared_with_me_is_hidden_when_the_provider_cannot_do_it() {
    let mut state = furnished();
    state.capabilities = vec![Capability::Files];
    let (mut h, _fake) = harness(state);
    open_page(&mut h);
    assert!(h.query_by_label("My files").is_some());
    assert!(h.query_by_label("Shared with me").is_none(), "a feature the provider does not have is hidden, not shown empty");
    assert!(h.query_by_label("Team.docx").is_none());
}

#[test]
fn opening_a_cloud_document_loads_its_text_and_remembers_where_it_came_from() {
    let (mut h, fake) = harness(furnished());
    open_page(&mut h);
    click(&mut h, "Report.docx");
    assert_eq!(body(&h), "Forty two", "the bytes went through the ordinary docx path");
    assert!(!h.state().ui.backstage, "a document that opened shows the page, not the File view");
    let origin = status(&mut h)["origin"].clone();
    assert_eq!(origin["id"], "/Report.docx", "the opaque handle is kept for the save");
    assert_eq!(origin["etag"], "v1", "so is the version it was read at");
    assert_eq!(origin["room"], "room:/Report.docx", "and the collaboration identity, which is not the handle");
    assert_eq!(fake.state.lock().unwrap().reads, vec!["/Report.docx".to_owned()]);
    assert!(h.state().session.path.is_none(), "a cloud document has no file on this computer");
    assert_eq!(h.state().title_stem(), "Report", "the window is named after the file in the cloud");
}

#[test]
fn save_writes_back_under_the_version_it_was_opened_at() {
    let (mut h, fake) = harness(furnished());
    open_page(&mut h);
    click(&mut h, "Report.docx");
    h.state_mut().run("text.insert", json!({"text": "More. "})).unwrap();
    h.run_steps(2);
    assert!(h.state().session.dirty);
    h.state_mut().run("file.save", json!({})).unwrap();
    h.run_steps(4);
    let state = fake.state.lock().unwrap();
    assert_eq!(state.writes.len(), 1, "one save, one write");
    assert_eq!(state.writes[0].0, "/Report.docx");
    assert_eq!(state.writes[0].1, Some("v1".to_owned()), "the etag it was read at is the condition");
    assert!(state.writes[0].2 > 0);
    drop(state);
    assert!(!h.state().session.dirty, "the version that reached the service is the saved one");
    assert_eq!(status(&mut h)["origin"]["etag"], "v1+", "and the document moves on to the version the service returned");
    // The stored copy really changed.
    let stored = fake.state.lock().unwrap().find("/Report.docx").unwrap().1.clone();
    let reread = wordcraft_engine::io::open_bytes("Report.docx", &stored).unwrap();
    assert!(reread.plain_text(StoryRef::Body).starts_with("More. Forty two"));
}

#[test]
fn a_save_that_would_overwrite_somebody_else_is_refused_and_offers_a_reload() {
    let (mut h, fake) = harness(furnished());
    open_page(&mut h);
    click(&mut h, "Report.docx");
    h.state_mut().run("text.insert", json!({"text": "Mine. "})).unwrap();
    h.run_steps(2);
    // Somebody else saves first.
    fake.state.lock().unwrap().overwrite("/Report.docx", document_bytes("Report.docx", "Theirs"));
    h.state_mut().run("file.save", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(fake.state.lock().unwrap().writes.len(), 1, "a refused save is never retried");
    let stored = fake.state.lock().unwrap().find("/Report.docx").unwrap().1.clone();
    let reread = wordcraft_engine::io::open_bytes("Report.docx", &stored).unwrap();
    assert_eq!(reread.plain_text(StoryRef::Body), "Theirs", "their work is still there");
    assert_eq!(status(&mut h)["conflict"]["name"], "Report.docx");
    assert!(h.query_by_label("This document was not saved").is_some(), "the user is told, and asked what to do");
    assert!(body(&h).starts_with("Mine. "), "their own edit is still open");
    assert!(h.state().session.dirty, "and still unsaved");
    // Reloading is explicit, and only then are the local edits replaced.
    click(&mut h, "Reload from the cloud");
    assert_eq!(body(&h), "Theirs");
    assert!(!h.state().session.dirty);
    assert_eq!(status(&mut h)["conflict"], Json::Null);
    assert_eq!(status(&mut h)["origin"]["id"], "/Report.docx", "the reloaded document still belongs to the same entry");
    assert_eq!(fake.state.lock().unwrap().writes.len(), 1, "and still nothing was written");
}

#[test]
fn save_as_offers_the_cloud_account_beside_this_computer() {
    let (mut h, fake) = harness(furnished());
    h.state_mut().run("text.insert", json!({"text": "Plan. "})).unwrap();
    h.state_mut().run("ui.backstage", json!({"value": true, "page": "saveAs"})).unwrap();
    h.run_steps(4);
    assert!(h.query_by_label("This PC").is_some(), "the local destination is still there");
    assert!(h.query_by_label("Ada Lovelace").is_some(), "and the cloud account beside it");
    click(&mut h, "Ada Lovelace");
    click(&mut h, "Reports");
    h.state_mut().cloud.save_name = "Plan".to_owned();
    h.run_steps(2);
    click(&mut h, "Save to the cloud");
    let state = fake.state.lock().unwrap();
    assert_eq!(state.creates.len(), 1);
    assert_eq!(state.creates[0].0, "/Reports", "into the folder that was picked");
    assert_eq!(state.creates[0].1, "Plan.docx", "a name typed without an extension is still a document");
    drop(state);
    assert!(!h.state().session.dirty, "the new document belongs to the cloud copy now");
    assert_eq!(status(&mut h)["origin"]["id"], "/Reports/Plan.docx");
    // Save now goes back to that entry, with no further questions.
    h.state_mut().run("text.insert", json!({"text": "Again. "})).unwrap();
    h.state_mut().run("file.save", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(fake.state.lock().unwrap().writes[0].0, "/Reports/Plan.docx");
}

#[test]
fn no_account_yet_is_an_offer_to_link_and_never_an_error() {
    let mut state = furnished();
    state.linked = false;
    let (mut h, fake) = harness(state);
    open_page(&mut h);
    assert!(h.state().status_msg.is_none(), "not having linked an account yet is not a failure");
    assert_eq!(status(&mut h)["link"], "offered");
    assert_eq!(status(&mut h)["accountError"], Json::Null);
    click(&mut h, "Link a cloud account");
    assert_eq!(fake.opened_urls.lock().unwrap().len(), 1, "the browser is opened for them");
    assert_eq!(status(&mut h)["link"]["waiting"], "https://cloud.example/link/abc");
    // They finish in the browser, and the next poll finds the account there.
    fake.state.lock().unwrap().linked = true;
    h.state_mut().cloud.link =
        Some(cloud::Link::Waiting { url: "https://cloud.example/link/abc".to_owned(), handle: Some("abc".to_owned()), next_poll: 0.0 });
    h.run_steps(6);
    assert_eq!(status(&mut h)["link"], Json::Null);
    assert_eq!(status(&mut h)["account"]["display"], "Ada Lovelace");
    assert!(h.query_by_label("Report.docx").is_some(), "and the files are listed without being asked again");
}

#[test]
fn a_read_only_share_is_never_saved_over() {
    let mut state = furnished();
    if let Some(slot) = state.shared.first_mut() {
        slot.0.writable = false;
    }
    let (mut h, fake) = harness(state);
    open_page(&mut h);
    click(&mut h, "Team.docx");
    assert_eq!(body(&h), "Ninety nine");
    // Save has nowhere of its own to go, so it asks, exactly as it does for a document that has
    // never been saved.
    h.state_mut().run("file.save", json!({})).unwrap();
    h.run_steps(3);
    assert!(fake.state.lock().unwrap().writes.is_empty(), "nothing was written back over somebody else's file");
    let refused = h.state_mut().run("cloud.save", json!({})).unwrap_err();
    assert!(refused.contains("reading only"), "and saying why names the fix: {refused}");
}

#[test]
fn a_local_document_still_saves_to_disk_with_a_cloud_service_wired() {
    let dir = std::env::temp_dir().join(format!("wordcraft-cloud-local-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Local.docx");
    let (mut h, fake) = harness(furnished());
    h.state_mut().run("text.insert", json!({"text": "Local. "})).unwrap();
    h.state_mut().run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
    h.run_steps(3);
    assert!(path.exists(), "the local Save path is untouched");
    assert!(fake.state.lock().unwrap().writes.is_empty());
    // And Save keeps going to that file afterwards.
    h.state_mut().run("text.insert", json!({"text": "Again. "})).unwrap();
    h.state_mut().run("file.save", json!({})).unwrap();
    h.run_steps(3);
    assert!(fake.state.lock().unwrap().writes.is_empty());
    // The caret stayed where the first insert left it, so the second one landed after it.
    let reread = wordcraft_engine::io::open_bytes("Local.docx", &std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(reread.plain_text(StoryRef::Body), "Local. Again. Draft", "the second save reached the same file");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_can_be_opened_by_name_from_the_control_channel() {
    let (mut h, _fake) = harness(furnished());
    h.state_mut().run("cloud.open", json!({})).unwrap();
    h.run_steps(4);
    h.state_mut().run("cloud.openFile", json!({"name": "Report.docx"})).unwrap();
    h.run_steps(4);
    assert_eq!(body(&h), "Forty two");
    let refused = h.state_mut().run("cloud.openFile", json!({"name": "accounts.pdf"})).unwrap_err();
    assert!(refused.contains("cannot open"), "{refused}");
    assert!(h.state_mut().run("cloud.openFile", json!({"name": "nothing.docx"})).is_err());
}

#[test]
fn names_and_paths_are_handled_as_a_user_types_them() {
    assert_eq!(cloud::document_name(" Plan "), "Plan.docx");
    assert_eq!(cloud::document_name("Plan.docx"), "Plan.docx");
    assert_eq!(cloud::document_name("Plan.odt"), "Plan.odt");
    assert_eq!(cloud::document_name("Plan.v2"), "Plan.v2.docx", "an unknown suffix is part of the name");
    assert_eq!(cloud::document_name("/a/b/Plan"), "Plan.docx", "a pasted path cannot become a folder");
    assert_eq!(cloud::document_name("   "), "");
    assert_eq!(cloud::parent_of(""), None);
    assert_eq!(cloud::parent_of("/Reports"), Some(String::new()));
    assert_eq!(cloud::parent_of("/Reports/2026"), Some("/Reports".to_owned()));
}

#[test]
fn working_together_opens_where_the_provider_says_to_go() {
    let mut state = furnished();
    state.capabilities.push(Capability::Conversation);
    let (mut h, fake) = harness(state);
    h.state_mut().run("cloud.open", json!({})).unwrap();
    h.run_steps(4);
    h.state_mut().run("cloud.openFile", json!({"name": "Report.docx"})).unwrap();
    h.run_steps(4);

    assert!(cloud::can_converse(h.state()), "the capability was reported");
    cloud::work_together(h.state_mut());
    h.run_steps(4);

    let opened = fake.opened_urls.lock().unwrap().clone();
    assert_eq!(opened, vec!["https://cloud.example/call/room".to_owned()]);
    assert_eq!(status(&mut h)["status"], json!("Opened in Talk"));
}

#[test]
fn a_document_that_was_never_in_the_cloud_is_asked_to_be_saved_there_first() {
    let mut state = furnished();
    state.capabilities.push(Capability::Conversation);
    let (mut h, fake) = harness(state);

    // The document the harness starts with was never in the cloud. There is no handle to key a
    // conversation on.
    cloud::work_together(h.state_mut());
    h.run_steps(4);

    assert!(fake.opened_urls.lock().unwrap().is_empty(), "nowhere to go, so nothing opened");
    assert_eq!(status(&mut h)["status"], json!("Save this document to the cloud first"));
}

#[test]
fn a_provider_with_nowhere_to_talk_never_offers_it() {
    // furnished() reports no conversation capability.
    let (h, _fake) = harness(furnished());
    assert!(!cloud::can_converse(h.state()), "the control has nothing to draw");
}

// ---------------------------------------------------------------- WordCraft's own answers

/// A window holds one document, so opening a cloud file replaces the one in it. Unsaved changes
/// are asked about first, and the document then opens frames later, after the pages have already
/// been answered. It still has to end up knowing where it came from.
#[test]
fn opening_over_unsaved_changes_asks_first_and_still_remembers_the_entry() {
    let (mut h, fake) = harness(furnished());
    h.state_mut().run("text.insert", json!({"text": "Unsaved. "})).unwrap();
    h.run_steps(2);
    assert!(h.state().session.dirty);
    open_page(&mut h);
    click(&mut h, "Report.docx");
    // The bytes were read, and the prompt is holding them.
    assert_eq!(fake.state.lock().unwrap().reads, vec!["/Report.docx".to_owned()]);
    assert!(body(&h).starts_with("Unsaved. "), "nothing was thrown away without asking");
    assert_eq!(status(&mut h)["origin"], Json::Null, "and nothing claims to have come from the cloud yet");
    // Don't Save: the open the prompt was holding goes ahead.
    h.state_mut().run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    h.run_steps(4);
    assert_eq!(body(&h), "Forty two");
    assert_eq!(status(&mut h)["origin"]["id"], "/Report.docx", "the document knows where it came from");
    assert!(h.state().session.path.is_none());
    // Which means Save goes back there.
    h.state_mut().run("text.insert", json!({"text": "Edited. "})).unwrap();
    h.state_mut().run("file.save", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(fake.state.lock().unwrap().writes[0].0, "/Report.docx");
}

/// Cancelling the prompt keeps the document that is open, and leaves nothing half-claimed behind
/// it: the next document to open is not the one the cloud read was about.
#[test]
fn a_cancelled_prompt_leaves_no_claim_on_the_next_document() {
    let (mut h, fake) = harness(furnished());
    h.state_mut().run("text.insert", json!({"text": "Unsaved. "})).unwrap();
    h.run_steps(2);
    open_page(&mut h);
    click(&mut h, "Report.docx");
    h.state_mut().run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
    h.run_steps(4);
    assert!(body(&h).starts_with("Unsaved. "), "their document is still open");
    assert_eq!(status(&mut h)["origin"], Json::Null);
    // A document that replaces it now belongs to nobody but itself. New asks about the unsaved
    // changes in its turn, as it always did.
    h.state_mut().run("file.new", json!({})).unwrap();
    h.state_mut().run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    h.run_steps(4);
    assert_eq!(body(&h).trim(), "", "a blank document");
    assert_eq!(status(&mut h)["origin"], Json::Null, "and it did not inherit the cloud entry");
    assert!(fake.state.lock().unwrap().writes.is_empty());
}

/// Answering Save to the prompt sends a cloud document back to the cloud. It has a destination
/// already, so nothing asks where to put it. The command the prompt was holding up waits, because
/// a cloud save has not happened yet when the answer returns.
#[test]
fn answering_save_sends_a_cloud_document_back_to_the_cloud() {
    let (mut h, fake) = harness(furnished());
    open_page(&mut h);
    click(&mut h, "Report.docx");
    h.state_mut().run("text.insert", json!({"text": "Edited. "})).unwrap();
    h.run_steps(2);
    // Anything that would replace the document asks about the unsaved changes first.
    h.state_mut().run("file.new", json!({})).unwrap();
    h.run_steps(2);
    h.state_mut().run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    // The bytes are on their way before the answer returns, and nowhere else was asked about.
    let state = fake.state.lock().unwrap();
    assert_eq!(state.writes.len(), 1);
    assert_eq!(state.writes[0].0, "/Report.docx");
    drop(state);
    // Until they land, the command the prompt was holding up does not happen, and the user is
    // told why rather than being left with a button that did nothing.
    let said = |h: &Harness| h.state().status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
    assert!(said(&h).contains("still being saved"), "{}", said(&h));
    h.run_steps(4);
    assert!(body(&h).starts_with("Edited. "), "the document is still the one that was open");
    assert!(said(&h).contains("Saved Report.docx"), "once the save lands the line says so: {}", said(&h));
    assert!(!h.state().session.dirty, "and the document is saved");
}

/// The bytes are written in the format the file name says, which for a save that writes back is
/// the name the entry already has.
#[test]
fn a_document_opened_as_opendocument_is_written_back_as_opendocument() {
    let mut state = furnished();
    state.files.push((file_entry("/Minutes.odt", None), document_bytes("Minutes.odt", "Attending")));
    let (mut h, fake) = harness(state);
    open_page(&mut h);
    click(&mut h, "Minutes.odt");
    assert_eq!(body(&h), "Attending");
    h.state_mut().run("text.insert", json!({"text": "Apologies. "})).unwrap();
    h.state_mut().run("file.save", json!({})).unwrap();
    h.run_steps(4);
    let stored = fake.state.lock().unwrap().find("/Minutes.odt").unwrap().1.clone();
    // An ODF package, not a Word one. The mimetype entry is stored uncompressed and first, so
    // the media type reads straight out of the bytes.
    let odf = b"application/vnd.oasis.opendocument.text";
    assert!(stored.windows(odf.len()).any(|w| w == odf), "the bytes are an ODF package, not a Word one");
    let reread = wordcraft_engine::io::open_bytes("Minutes.odt", &stored).unwrap();
    assert!(reread.plain_text(StoryRef::Body).starts_with("Apologies. Attending"));
}

/// Writes one PNG per state to the directory in `WORDCRAFT_CLOUD_RENDERS`.
///
/// Skipped without it. The renders are review material for a human being, and the behaviour they
/// illustrate is asserted by the tests above. Starting wgpu in ordinary CI would buy nothing.
#[test]
fn renders_for_review() {
    let Ok(into) = std::env::var("WORDCRAFT_CLOUD_RENDERS") else {
        eprintln!("set WORDCRAFT_CLOUD_RENDERS to a directory to write renders");
        return;
    };
    let into = std::path::PathBuf::from(into);
    std::fs::create_dir_all(&into).expect("the render directory must be writable");

    let save = |h: &mut Harness, name: &str| {
        h.run_steps(8);
        let image = h.render().expect("an offscreen renderer is required to write renders");
        let path = into.join(format!("wordcraft-{name}.png"));
        image.save(&path).expect("the render directory must be writable");
        eprintln!("wrote {}", path.display());
    };

    // The file list, where every document carries an overflow menu. One provider that does
    // everything, built twice. Two windows are needed because one of them has a document open and
    // the other is browsing.
    let talkative = || {
        let mut state = furnished();
        state.capabilities.push(Capability::Conversation);
        state
    };
    let (mut h, _fake) = rendering(talkative());
    open_page(&mut h);
    save(&mut h, "open-page");

    // The same page with one of those menus open.
    h.get_all_by_role_and_label(egui::accesskit::Role::Button, "More options").next().expect("a row offers its menu").click();
    save(&mut h, "open-page-overflow");

    // The Save As page, with the cloud account picked as the destination.
    let (mut h, _fake) = rendering(talkative());
    h.state_mut().run("ui.backstage", json!({"value": true, "page": "saveAs"})).unwrap();
    h.run_steps(4);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Ada Lovelace").click();
    save(&mut h, "save-as-page");

    // The title bar: the account, who else is here, and Share as a split button with the chevron
    // pressed.
    let (mut h, _fake) = rendering(talkative());
    h.state_mut().run("cloud.open", json!({})).unwrap();
    h.run_steps(4);
    h.state_mut().run("cloud.openFile", json!({"name": "Report.docx"})).unwrap();
    h.run_steps(4);
    let peers = ["Dana Okafor", "Priya Raman", "Tomás Alvarez", "Wen Li", "Sam Iredale", "Rosa Beltrán"];
    h.state_mut().presence.set(peers.iter().enumerate().map(|(n, who)| craft_ui::presence::Peer::new(format!("peer-{n}"), *who)).collect());
    save(&mut h, "title-bar-presence");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "More share options").click();
    save(&mut h, "share-split-menu");

    // And the dialog the primary half opens, with real access on it.
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Share").click();
    save(&mut h, "share-dialog");
}
