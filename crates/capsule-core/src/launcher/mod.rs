pub mod folders;
pub mod haystack;
pub mod history;
pub mod hotkey;
pub mod input;
pub mod query;
pub mod rank;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    App,
    Window,
    Command,
    Folder,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::App => "App",
            Source::Window => "Window",
            Source::Command => "Command",
            Source::Folder => "File",
        }
    }

    fn tag(self) -> char {
        match self {
            Source::App => 'a',
            Source::Window => 'w',
            Source::Command => 'c',
            Source::Folder => 'f',
        }
    }

    fn from_tag(c: char) -> Option<Source> {
        [Source::App, Source::Window, Source::Command, Source::Folder].into_iter().find(|s| s.tag() == c)
    }
}
