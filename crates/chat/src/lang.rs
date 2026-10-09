//! Chat language: every chat-visible string of the hub, the gate and the pane, in one table.
//! English is the default; `WORDCRAFT_CHAT_LANG=pt` gives Portuguese. Authority never depends on
//! these strings: it comes from a message's `role`.

/// The chat's language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    Pt,
}

/// A chat-visible string (arguments are already one-line text).
#[derive(Clone, Debug)]
pub enum Text<'a> {
    /// The display `from` of the owner's messages.
    OwnerFrom,
    /// The display `from` of system lines.
    SystemFrom,
    Joined(&'a str),
    Removed(&'a str),
    /// Another document is open (its file name).
    Document(&'a str),
    /// The name shown for a document without a file.
    NewDocument,
    /// Where the chat log is.
    LogAt(&'a str),
    Formatted { who: &'a str, command: &'a str },
    Judged { who: &'a str, accepted: bool, chars: usize, authors: &'a str },
    Resolved { who: &'a str, reopened: bool, author: &'a str },
    /// A member's reject that would leave new paragraphs split.
    RejectParagraphs,
    /// `select.owner` when the owner has only a caret.
    OwnerNoSelection,
    InviteLine { instance: &'a str, code: &'a str, handle: &'a str },
    InviteFailed(&'a str),
    ChatOff,
    NameNoAt,
    LogNotSaved(&'a str),
    InMemory,
    Brake,
    InputHint,
    NotSent(&'a str),
}

impl Lang {
    /// `en` or `pt` (also `pt-PT`, `pt_BR`…); anything else is English.
    pub fn parse(code: &str) -> Lang {
        let c = code.trim().to_ascii_lowercase();
        if c == "pt" || c.starts_with("pt-") || c.starts_with("pt_") { Lang::Pt } else { Lang::En }
    }

    /// From `WORDCRAFT_CHAT_LANG` (English when unset).
    pub fn from_env() -> Lang {
        std::env::var("WORDCRAFT_CHAT_LANG").map(|v| Lang::parse(&v)).unwrap_or_default()
    }

    /// The code clients get (`chat.join` result).
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Pt => "pt",
        }
    }

    pub fn text(self, t: Text) -> String {
        use Lang::{En, Pt};
        match (self, t) {
            (En, Text::OwnerFrom) => "OWNER".into(),
            (Pt, Text::OwnerFrom) => "DONO".into(),
            (En, Text::SystemFrom) => "SYSTEM".into(),
            (Pt, Text::SystemFrom) => "SISTEMA".into(),
            (En, Text::Joined(h)) => format!("{h} joined the chat"),
            (Pt, Text::Joined(h)) => format!("{h} entrou no chat"),
            (En, Text::Removed(h)) => format!("{h} was removed"),
            (Pt, Text::Removed(h)) => format!("{h} foi removido"),
            (En, Text::Document(n)) => format!("document: {n}"),
            (Pt, Text::Document(n)) => format!("documento: {n}"),
            (En, Text::NewDocument) => "new".into(),
            (Pt, Text::NewDocument) => "novo".into(),
            (En, Text::LogAt(p)) => format!("chat saved in {p}"),
            (Pt, Text::LogAt(p)) => format!("chat guardado em {p}"),
            (En, Text::Formatted { who, command }) => format!("{who} formatted: {command}"),
            (Pt, Text::Formatted { who, command }) => format!("{who} formatou: {command}"),
            (En, Text::Judged { who, accepted, chars, authors }) => {
                let verb = if accepted { "accepted" } else { "rejected" };
                let what = if chars == 1 { "character" } else { "characters" };
                format!("{who} {verb} {chars} {what} from {authors}")
            }
            (Pt, Text::Judged { who, accepted, chars, authors }) => {
                let verb = if accepted { "aceitou" } else { "rejeitou" };
                let what = if chars == 1 { "carácter" } else { "caracteres" };
                format!("{who} {verb} {chars} {what} de {authors}")
            }
            (En, Text::Resolved { who, reopened, author }) => format!("{who} {} the comment by {author}", if reopened { "reopened" } else { "resolved" }),
            (Pt, Text::Resolved { who, reopened, author }) => format!("{who} {} o comentário de {author}", if reopened { "reabriu" } else { "resolveu" }),
            (En, Text::RejectParagraphs) => "rejecting new paragraphs: ask the OWNER".into(),
            (Pt, Text::RejectParagraphs) => "rejeitar parágrafos novos: pede ao DONO".into(),
            (En, Text::OwnerNoSelection) => "the OWNER has no text selected: this is the paragraph at the OWNER's caret".into(),
            (Pt, Text::OwnerNoSelection) => "o DONO não tem texto selecionado: é o parágrafo onde está o cursor dele".into(),
            (En, Text::InviteLine { instance, code, handle }) => format!("Join the WordCraft chat: wordcraft-chat join {instance}:{code} --as {handle}"),
            (Pt, Text::InviteLine { instance, code, handle }) => format!("Entra no chat do WordCraft: wordcraft-chat join {instance}:{code} --as {handle}"),
            (En, Text::InviteFailed(e)) => format!("Invite failed: {e}"),
            (Pt, Text::InviteFailed(e)) => format!("O convite falhou: {e}"),
            (En, Text::ChatOff) => "Chat is off: start WordCraft with --control.".into(),
            (Pt, Text::ChatOff) => "O chat está desligado: arranca o WordCraft com --control.".into(),
            (En, Text::NameNoAt) => "your name cannot start with @".into(),
            (Pt, Text::NameNoAt) => "o teu nome não pode começar por @".into(),
            (En, Text::LogNotSaved(e)) => format!("Chat log not saved: {e}"),
            (Pt, Text::LogNotSaved(e)) => format!("A conversa não foi guardada: {e}"),
            (En, Text::InMemory) => "The chat is kept in memory until you save the document.".into(),
            (Pt, Text::InMemory) => "A conversa fica em memória até gravares o documento.".into(),
            (En, Text::Brake) => "Agents wait for you (8 messages in a row).".into(),
            (Pt, Text::Brake) => "Os agentes esperam por ti (8 mensagens seguidas).".into(),
            (En, Text::InputHint) => "Message (@claude …), Enter to send".into(),
            (Pt, Text::InputHint) => "Mensagem (@claude …), Enter envia".into(),
            (En, Text::NotSent(e)) => format!("Not sent: {e}"),
            (Pt, Text::NotSent(e)) => format!("Não enviada: {e}"),
        }
    }
}
