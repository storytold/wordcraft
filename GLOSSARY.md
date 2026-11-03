# WordCraft

A clean-room, Rust-native word processor. These are the words its interface, docs and code use for
the same things.

## Languages and proofing

**Interface language**:
The language of WordCraft's own menus, buttons and messages.
_Avoid_: Display language, UI locale

**Proofing language**:
The language a piece of text is marked as, which decides how its spelling, grammar and hyphenation
are checked.
_Avoid_: Text language, editing language, document language

**Language pack**:
A downloadable bundle of a language's spelling dictionary and hyphenation patterns.
_Avoid_: Dictionary download, language data (that is the embedded data)

**Your languages**:
The proofing languages Options › Language lists: each English variant in use (the default proofing
language and any in the open document), every language with an installed
Language pack, and every proofing language used in the open document.
_Avoid_: Authoring languages, preferred languages

**Default proofing language**:
The proofing language new text gets, set with Set As Default; without one, the interface language.
_Avoid_: Preferred language

**Proofing status**:
Where a language's proofing comes from: built in, an installed Language pack, your own dictionary,
available to install, or not available.

**Custom dictionary**:
The words you added with Add to Dictionary, accepted in every language.
_Avoid_: User dictionary (in the interface)

**Your own dictionaries folder**:
A folder of Hunspell dictionaries you supply, used before Language packs and the system's.
_Avoid_: Dictionaries folder, custom dictionaries
