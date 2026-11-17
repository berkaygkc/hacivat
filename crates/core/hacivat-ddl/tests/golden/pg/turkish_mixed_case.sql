CREATE TABLE shop."MÜŞTERİ" (
    "Ad" text NOT NULL,
    "order" integer,
    "müşteri_no" bigint,
    "ılık" boolean,
    CONSTRAINT "MÜŞTERİ_pkey" PRIMARY KEY ("müşteri_no")
);

COMMENT ON TABLE shop."MÜŞTERİ" IS 'Müşteri kayıtları — İstanbul şubesi';

COMMENT ON COLUMN shop."MÜŞTERİ"."ılık" IS 'Sıcaklık "derece" -- yorum değil
ikinci satır';
