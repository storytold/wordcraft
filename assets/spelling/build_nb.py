#!/usr/bin/env python3
"""Extract current Bokmål or Nynorsk forms from Språkbanken's 2022-02-01 Ordbank dump.

Usage: python3 assets/spelling/build_nb.py /path/to/20220201_norsk_ordbank_nob_2005.tar.gz [nb]
       python3 assets/spelling/build_nb.py /path/to/20220201_norsk_ordbank_nno_2012.tar.gz nn
Sources: https://www.nb.no/sprakbanken/ressurskatalog/oai-nb-no-sbr-5/
         https://www.nb.no/sprakbanken/ressurskatalog/oai-nb-no-sbr-41/
The derived word list remains CC BY 4.0. This extraction script is MIT OR Apache-2.0.
"""
import hashlib
import sys
import tarfile
import unicodedata
import zlib
from pathlib import Path

language = sys.argv[2] if len(sys.argv) > 2 else 'nb'
if language not in ('nb', 'nn'):
    raise SystemExit('Language must be nb or nn')
archive = Path(sys.argv[1])
forms = set()
with tarfile.open(archive) as source:
    table = source.extractfile('fullformsliste.txt' if language == 'nb' else 'fullformer_2012.txt')
    next(table)
    for raw in table:
        fields = raw.decode('iso-8859-1').rstrip('\r\n').split('\t')
        if len(fields) < 9 or fields[7] != '4000' or fields[8] != 'normert':
            continue
        word = unicodedata.normalize('NFC', fields[2]).lower()
        if (2 <= len(word) <= 120 and word[0].isalpha() and word[-1].isalpha()
                and all(c.isalpha() or c in "-'" for c in word)):
            forms.add(word)
words = sorted(forms)
raw = (('WCNB1\n' if language == 'nb' else 'WCNN1\n') + '\n'.join(words) + '\n').encode('utf-8')
compressor = zlib.compressobj(level=9, wbits=-15)
output = compressor.compress(raw) + compressor.flush()
Path(__file__).with_name(f'{language}-NO.dic').write_bytes(output)
print(f'{len(words)} forms; {len(raw)} bytes decoded; {len(output)} bytes compressed')
print(f'Source SHA-256: {hashlib.sha256(archive.read_bytes()).hexdigest()}')
