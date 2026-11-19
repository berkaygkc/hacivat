# pg17wrap (M4 Task 1c, R-M4-43; ver/01 #pg-xmin-freeze): postgres:17 whose cluster jumps
# past 2^31 transaction ids at its first start, so the incremental-introspection tests see
# frozen rows with a negative `age(xmin)` and a watermark span near wraparound (R-M4-8).
# init.sh initialises the cluster itself, then hands over to the image's own entrypoint, which
# finds PGDATA initialised and only starts the server. Built and started by controller step C-6.
FROM postgres:17@sha256:846a7ddaa67a4b354ae1d21087b683901aa6113a4ed626d3787cfbf6c85e3e07
COPY --chmod=0755 init.sh /usr/local/bin/hv-pg-wrap.sh
ENTRYPOINT ["hv-pg-wrap.sh"]
CMD ["postgres"]
