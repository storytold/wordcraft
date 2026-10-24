//! Hand-written synthetic Slovak test data. No words, rules or suggestion tables were copied
//! from a system dictionary. This is a small fixture, not a redistributable Slovak dictionary.

pub const AFF: &str = "SET UTF-8
FLAG UTF-8
FORBIDDENWORD F
TRY aeiouyáäéíóôúýčďľĺňŕšťžntklmprsvbhzj
KEY xn|qš
MAP 14
MAP aáä
MAP eé
MAP ií
MAP oóô
MAP uú
MAP yý
MAP cč
MAP sš
MAP zž
MAP dď
MAP lľĺ
MAP nň
MAP rŕ
MAP tť
REP 1
REP pj pi
SFX A Y 3 # synthetic neuter declension
SFX A o a o
SFX A o e o
SFX A o u o
SFX Z Y 4 # synthetic feminine declension
SFX Z a y a
SFX Z a e a
SFX Z a ou a
SFX Z a u a
SFX R Y 2
SFX R 0 y .
SFX R 0 o/C .
SFX C N 1
SFX C o ejší o
SFX P N 2
SFX P a e a
SFX P a u a
PFX X Y 1
PFX X 0 ne .
";

pub const DIC: &str = "26
mesto/A
žena/Z
rýchl/RX
Bratislava/P
škola
počítač
piatok
ďakujeme
kôň
stôl
päť
mäso
dĺžka
vŕba
ľúbiť
ťava
dzbán
džús
chlieb
móda
bielo-čierny
nie
je
sa
to
zqzakázané/F
";

pub fn install() {
    super::install_dictionary(AFF, DIC).unwrap();
}
