// Original WordCraft test data, MPL-2.0. This is a deliberately small synthetic
// dictionary for the vendored unit tests, not the upstream GPL en_US dictionary
// and not a general English spelling resource.
pub const AFF: &str = "SET UTF-8
TRY abcdefghijklmnopqrstuvwxyz
REP 1
REP exsize excise
ICONV 1
ICONV ’ '
PFX R Y 1
PFX R 0 re .
PFX U Y 1
PFX U 0 un .
SFX V N 2
SFX V e ive e
SFX V 0 ive [^e]
SFX D Y 1
SFX D 0 ed .
SFX G Y 1
SFX G 0 ing .
COMPOUNDMIN 1
COMPOUNDRULE 1
COMPOUNDRULE n*o
ONLYINCOMPOUND X
";
pub const DIC: &str = "47
hello
world
adventure
any
anyway
anywhere
ace
concuss/V
regenerate/V
route/R
drink
Achilles
BBQ
can't
earth/UD
Alice
RSVP
ANSI
excise
fiver
faker
favor
example
need
dream
banana
execute
0/nX
1/nX
2/nX
3/nX
4/nX
5/nX
6/nX
7/nX
8/nX
9/nX
0th/oX
1th/oX
2th/oX
3th/oX
4th/oX
1st/oX
2nd/oX
3rd/oX
5th/oX
6th/oX
";
