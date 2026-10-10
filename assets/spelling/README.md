# Norwegian spelling data

`nb-NO.dic` is derived from **Norsk ordbank – bokmål 2005**, the snapshot dated
2022-02-01 published by Nasjonalbiblioteket / Språkbanken. The source catalogue:
https://www.nb.no/sprakbanken/ressurskatalog/oai-nb-no-sbr-5/

Source archive:
https://www.nb.no/sbfil/leksikalske_databaser/ordbank/20220201_norsk_ordbank_nob_2005.tar.gz
SHA-256: `57d91cb3f17b85befa50a3b56fcaed9800ca665b39486fe97b668add914d5f60`

The original data and the derived dictionary are licensed under **CC BY 4.0**:
https://creativecommons.org/licenses/by/4.0/ (full text: `CC-BY-4.0.txt`).
No endorsement by the original rights holder is implied.

Changes: extracted the `OPPSLAG` column of `fullformsliste.txt` (ISO-8859-1), kept
current (`TILDATO=4000`) standard (`NORMERING=normert`) forms, normalized to NFC
and lowercase, retained words of 2–120 characters containing letters and internal
hyphens/apostrophes, removed duplicates, sorted by Unicode code point, and
compressed the resulting UTF-8 text using raw DEFLATE. Result: 613,686 forms.
The format starts with `WCNB1\n`, followed by one word per line.

Rebuild with the original extraction script (MIT OR Apache-2.0):

```sh
python3 assets/spelling/build_nb.py /path/to/20220201_norsk_ordbank_nob_2005.tar.gz
```

The word list includes full inflected forms. It does not generate arbitrary new
compounds and is not a comprehensive Norwegian grammar checker. The application
embeds the data and never sends document text to an online service.

## Nynorsk

`nn-NO.dic` is derived from **Norsk ordbank – nynorsk 2012**, the snapshot dated
2022-02-01 published by Nasjonalbiblioteket / Språkbanken. The source catalogue:
https://www.nb.no/sprakbanken/ressurskatalog/oai-nb-no-sbr-41/

Source archive:
https://www.nb.no/sbfil/leksikalske_databaser/ordbank/20220201_norsk_ordbank_nno_2012.tar.gz
SHA-256: `32987494b77c54b5c3890e891b99e35d5a45a16643a5c1820cd5e02fd55b1b8d`

The original data and derived dictionary are **CC BY 4.0**, with the same license
text and attribution requirements as the Bokmål data. No endorsement is implied.
The same extraction rules are applied to `fullformer_2012.txt`: current standard
forms only, NFC/lowercase normalization, alphabetic forms with internal hyphens or
apostrophes, deduplication, sorting and raw DEFLATE compression. Result: **409,690
forms**, 4,798,896 decoded bytes, 1,099,870 compressed bytes. The header is `WCNN1\n`.
The list includes the 2012 spelling reform; it is separate from the Bokmål list.

```sh
python3 assets/spelling/build_nb.py /path/to/20220201_norsk_ordbank_nno_2012.tar.gz nn
```

Nynorsk has the same limitations: full inflected forms, no generation of arbitrary
new compounds, and only basic grammar checks. Each language's dictionary is loaded
lazily and never sends document text online.
