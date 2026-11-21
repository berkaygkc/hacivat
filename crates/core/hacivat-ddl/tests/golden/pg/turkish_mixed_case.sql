CREATE TABLE "MÜŞTERİ" (
    "Ad" text,
    "order" integer,
    "müşteri_no" integer NOT NULL,
    "ılık" boolean,
    CONSTRAINT "MÜŞTERİ_pkey" PRIMARY KEY ("müşteri_no")
);

COMMENT ON TABLE "MÜŞTERİ" IS 'Müşteri''s table -- don''t
edit';
