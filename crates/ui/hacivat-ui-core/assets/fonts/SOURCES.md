# Bundled fonts

All fonts are licensed under SIL Open Font License 1.1 (OFL-1.1). Static TTF files are used (not variable fonts).

| Family | Files | Release | Archive URL | Archive SHA-256 |
|---|---|---|---|---|
| Source Sans 3 | SourceSans3-{Regular,Medium,Semibold,Bold}.ttf (from `TTF/`) | 3.052R | https://github.com/adobe-fonts/source-sans/releases/download/3.052R/TTF-source-sans-3.052R.zip | 1b0dd1ec44b39f1dd98bbd153a1a3815f083639874ddee02c842bd601bad3d21 |
| Fragment Mono | FragmentMono-{Regular,Italic}.ttf (from `fonts/ttf/`) | 1.21 | https://github.com/weiweihuanghuang/fragment-mono/releases/download/1.21/fragment-mono-1.21.zip | 1d3b711facfb0621638dc650708f8a26dbe83b1efb4382a2438ac5eadb716658 |

## File SHA-256

```
4644c81b86ec9caaa76b634889968ed3c4f4f52f054855933acc7c2b21e53b0f  SourceSans3-Regular.ttf
a7af35c1cab5dd0b29d742f86f2edfdf83fa7ec213643e765c5c51a2dc24edbc  SourceSans3-Medium.ttf
a3f4f8dcf343a8f24dc61951de93f3ba1558b15cd250ba24af8a40e957081b7d  SourceSans3-Semibold.ttf
9214b9d95e4231c609802815c2646c98174e2102d0d37f88978a7f8e71006e6a  SourceSans3-Bold.ttf
e8beb5248b1d38bc24962c6d04bc49204248af5f6e1b3492fda17179a70f1d2d  FragmentMono-Regular.ttf
571b60b07c5f4bd33ecfc8ca874cdc29be6c58e231621d2acb2beea9855dd61c  FragmentMono-Italic.ttf
```

## Licenses

- `OFL-SourceSans3.md`: `LICENSE.md` from adobe-fonts/source-sans tag 3.052R (SHA-256 89ad2c4f66dd29127527493e729c31e731f111cf10faf5774c3db9275ed0c22c). Copyright 2010-2022 Adobe, Reserved Font Name "Source".
- `OFL.txt`: from the Fragment Mono 1.21 archive (SHA-256 26ddc9e68d9b59f546bf9cbb8b217954752771620590aa9a137b54fa4baf0218). Copyright 2024 The Fragment-Mono Project Authors.

## OpenType features (verified with Python fontTools 4.57.0)

Source Sans 3 Regular/Medium/Semibold/Bold GSUB contains `smcp` and `c2sc`. It does NOT contain a `tnum` feature: its default figures are already tabular (all digits advance 497 units in Regular) and it offers `pnum` for proportional. Requesting `tnum` is therefore harmless and keeps the default tabular figures.
