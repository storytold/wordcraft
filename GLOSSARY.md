# WordCraft

A clean-room, Rust-native word processor. These are the words its interface, docs and code use for
the same things.

## Languages and proofing

**Interface language**:
The language of WordCraft's own menus, buttons and messages.
_Avoid_: Display language, UI locale

**Proofing language**:
The language a piece of text is marked as, which decides how its spelling, grammar and hyphenation
are checked. A language is told apart from another by its tag (language, script, region, variant),
never by the name a list shows for it.
_Avoid_: Text language, editing language, document language

**Language pack**:
A downloadable bundle of a language's spelling dictionary and hyphenation patterns.
_Avoid_: Dictionary download, language data (that is the embedded data)

**Language list**:
The proofing languages offered in Set Proofing Language, Review › Language and the status bar's
menu: the languages WordCraft names, plus those of every installed (and, in Set Proofing Language,
every available) Language pack, each once.
_Avoid_: Language menu (that is only one of the places it is shown)

**Your languages**:
The proofing languages Options › Language lists: each English variant in use (the default proofing
language and any in the open document), every language with an installed
Language pack, and every proofing language used in the open document.
_Avoid_: Authoring languages, preferred languages

**Default proofing language**:
The proofing language new blank documents get, set with Set as default; without one, the interface
language.
_Avoid_: Preferred language

**Proofing status**:
Where a language's proofing comes from: built in, an installed Language pack, your own dictionary,
a system dictionary, available to install, or not available.

**Custom dictionary**:
The words you added with Add to Dictionary, accepted in every language.
_Avoid_: User dictionary (in the interface)

**Dictionaries folder**:
A folder of Hunspell dictionaries you supply, used before the system's (an installed Language pack
comes after it).
_Avoid_: Custom dictionaries
