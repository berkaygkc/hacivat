# pg17-tr (R17): postgres:17 plus the tr_TR.UTF-8 locale, so `lc_messages=tr_TR.UTF-8`
# (set in compose.yml) gives Turkish server messages from the bundled postgres-17.mo.
# One small layer; the official image documents this localedef recipe.
FROM postgres:17@sha256:846a7ddaa67a4b354ae1d21087b683901aa6113a4ed626d3787cfbf6c85e3e07
RUN localedef -i tr_TR -c -f UTF-8 -A /usr/share/locale/locale.alias tr_TR.UTF-8
