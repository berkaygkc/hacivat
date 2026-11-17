# M3c PO demosu: "Tamamlanmış" kontrol listesi

Bu liste M3c kapısının (Görev 45) yalnızca PO'nun yapabileceği kısmıdır: E1–E4 elle, gerçek bir
Mac'te, gerçek Docker sunucularına karşı, gerçek pencere ve gerçek klavyeyle. Her adımda şunlar
vardır: ne yapılacağı (tam SQL, metin ve tuşlar), beklenen ekran (uygulama metinleri `EN` / `TR`),
"geçti"nin ne demek olduğu ve adımı önceden doğrulayan otomatik testler. Sondaki tabloya her adımın
sonucunu yazın.

Kapı kaydı: `docs/benchmarks/m3c.md` (İngilizce). Bölüm 0 orada, PO'nun yapacağı her şeyi sıralar.

Önceden doğrulama adımın yerini tutmaz: uygulama testleri sahte sürücü ve başsız GPUI ile koşar.
Gerçek pencere, gerçek odak, IME, dar bölme ve ekran okuyucu yalnızca bu listeyle görülür.

**Terimler:** "transaction", "commit" ve "rollback" Türkçe metinlerde de İngilizce kalır (PO kararı,
M3b-28). Kısayollar VS Code alışkanlığındadır ve Ayarlar'dan değiştirilebilir.

**Bilinen ve onaylı iki durum (şaşırmayın):**
- **Yerel sıralama Türkçe harmanlamayla değil, kod noktasıyla yapılır** (M3c-06, onaylı). Başlıktaki
  ipucu bunu açıkça söyler. Türkçe harmanlama M5'te katalogdan okununca gelir. Türkçe esneklik
  **bulmada** vardır, sıralamada yoktur.
- **Başlığa tıklamak sıralamaz, sütunu seçer** (M3c-07, onaylı). Sıralama başlık/hücre menüsünden ya
  da komut paletinden (⌘K) yapılır.

## 0. Hazırlık

1. **Docker yığını** (`hacivat-m2a`) olduğu gibi ayakta; hiçbir kapsayıcı yeniden kurulmaz. Kontrol
   (yalnızca okur):

   ```
   cargo xtask docker status --set pr
   ```

   İyi görünüm: `m2a-pg17`, `m2a-mysql84` `healthy`. Docker testleri ya da ölçümler (adım 16)
   koşarken demo yapmayın.

2. **Uygulama** (release; adım 14'e kadar İngilizce):

   ```
   cargo run --release -p hacivat -- --lang en
   ```

3. **Test sunucuları** (yalnızca demo içindir; `docker/m2a/README.md`): kullanıcı `hv_test`, parola
   `hv_test_pw`, veritabanı `hv_test`; salt okunur kullanıcı `hv_ro`. Portlar (`127.0.0.1`): `pg17`
   54317, `mysql84` 33084.

4. **Profiller.** M3b demosundaki profiller duruyorsa onları kullanın; yoksa ⌘N, "Save and connect":

   | Profil | Motor | Port / dosya | Kullanıcı | Ortam | Adımlar |
   |---|---|---|---|---|---|
   | `pg17` | PostgreSQL | 54317 | `hv_test` | `dev` | 1–8, 10–13 |
   | `pg17-prod` | PostgreSQL | 54317 | `hv_test` | **`production` / `üretim`** | 14 |
   | `pg17-ro` | PostgreSQL | 54317 | `hv_ro` | `dev`, **Read-only** açık | 14 |
   | `mysql84` | MySQL | 33084 | `hv_test` | `dev` | 9 |
   | `sqlite-demo` | SQLite | yeni dosya `~/hv-demo-m3c.sqlite` | — | `dev` | 9 |

5. **Demo tabloları.** `pg17` konsolunda tümünü seçip ⌘↵:

   ```sql
   CREATE TABLE IF NOT EXISTS demo_sehir (id int PRIMARY KEY, ad text NOT NULL, bolge text, nufus int);
   DELETE FROM demo_sehir;
   INSERT INTO demo_sehir VALUES
     (1,'Zonguldak','Karadeniz',588000), (2,'Ankara','İç Anadolu',5800000), (3,'Çorum','Karadeniz',525000),
     (4,'İzmir','Ege',4500000), (5,'Iğdır','Doğu Anadolu',203000), (6,'ılıca','Doğu Anadolu',NULL),
     (7,'Şanlıurfa','Güneydoğu',2200000), (8,'Ödemiş','Ege',134000), (9,'Bursa','Marmara',3200000),
     (10,'Uşak','Ege',375000);
   CREATE TABLE IF NOT EXISTS demo_buyuk (id int, v text);
   CREATE TABLE IF NOT EXISTS demo_orders (id int PRIMARY KEY, city text NOT NULL, total numeric(12,2) NOT NULL);
   INSERT INTO demo_orders VALUES (1,'İstanbul',1234.56), (2,'İzmir',7.00), (3,'Şanlıurfa',1234567.50)
     ON CONFLICT (id) DO NOTHING;
   ```

   (`ılıca` bilerek küçük harfle başlar: `ı` harfinin sıralamadaki yerini göstermek için.)

6. Hatırlatmalar: Çalıştır ⌘↵, durdur ⌘., komut paleti ⌘K, geçmiş ⌘⇧H. Izgarada bul ⌘F (odak
   ızgaradayken), satıra git ⌘L, değer görüntüleyici ⇧↵, bağlam menüsü sağ tık ya da ⇧F10.
   Bir şey farklıysa adım numarasıyla not alın.

---

## 1. Sıralama: kod noktası, dürüst ipucu, sıralamadan sonra kopyalama (E1, E3)

**Yap:** `pg17` konsolunda:

```sql
SELECT * FROM demo_sehir;
```

`ad` başlığına sağ tıklayın → **Sort ascending** / **Artan sırala**. Başlıktaki oka gelip ipucunu
okuyun. Sonra `ad` başlığında sağ tık → **Clear sort** / **Sıralamayı kaldır**; `nufus` başlığında
sağ tık → **Sort descending** / **Azalan sırala**. Son olarak `ad` sütununun ilk üç
hücresini seçip ⌘C ve bir metin düzenleyiciye yapıştırın.

Karşılaştırma için aynı konsolda veritabanının Türkçe sırasını görün:

```sql
SELECT ad FROM demo_sehir ORDER BY ad COLLATE "tr-x-icu";
```

**İyi görünüm:**
- `ad` artan, **kod noktası** sırası: `Ankara, Bursa, Iğdır, Uşak, Zonguldak, Çorum, Ödemiş, İzmir,
  ılıca, Şanlıurfa`.
- İpucu: `Sort level 1, ascending. The column's collation is not known here, so text compares by
  code point.` / `Sıralama düzeyi 1, artan. Sütunun harmanlaması burada bilinmiyor; metin kod
  noktasına göre karşılaştırılır.`
- Veritabanının Türkçe sırası farklıdır: `Ankara, Bursa, Çorum, Iğdır, ılıca, İzmir, Ödemiş,
  Şanlıurfa, Uşak, Zonguldak`. Fark beklenen ve onaylı durumdur (M3c-06).
- `nufus` azalan: `ılıca` (NULL) en sonda.
- Başlığa düz tıklamak sütunu seçer, sıralamaz.
- Kopyalanan üç değer **ekrandaki sırayla** gelir.

**Geçti:** sıralama anında ve doğru (kod noktası), ipucu dürüst, kopya gösterilen sırada.

**Ön doğrulama:** `copy_after_sort_copies_what_is_shown`, `a_stale_sort_never_replaces_a_newer_one`,
`sort_of_a_million_rows_is_cancellable_and_off_thread` (`results_sort.rs`);
`code_point_order_without_a_mapped_collation` (`hacivat-values/tests/compare.rs`);
`sort_then_copy_copies_the_shown_order` (`cross_surface_m3c.rs`). Görüntü `sort-code-point-tip`.

## 2. Izgarada bul (Türkçe esnek), Satırları süz, Tüm satırları göster (E1, E3)

**Yap:** Adım 1'in sonucunda ızgaraya tıklayın, ⌘F. Bul çubuğunda **Turkish-tolerant** / **Türkçe
esnek** açık olsun. Sırayla yazın: `izmir`, sonra `igdir`, sonra `sanliurfa`, sonra `ege`. `ege`
yazılıyken **Filter rows** / **Satırları süz**'e basın; sonra **Show all rows** / **Tüm satırları
göster**. Son olarak `null` yazın.

**İyi görünüm:**
- Alan ipucu: `Find in the shown values` / `Görünen değerlerde bul`.
- `izmir` → `İzmir` bulunur (`1 of 1` / `1/1`); `igdir` → `Iğdır`; `sanliurfa` → `Şanlıurfa`.
- `ege` → 3 eşleşme (`1 of 3` / `1/3`); süzünce yalnız 3 satır: `Showing the 3 rows that match, of
  the 10 fetched. Rows not fetched yet aren't filtered.` / `Getirilen 10 satırdan eşleşen 3 satır
  gösteriliyor. Henüz getirilmeyen satırlar süzülmez.`
- Tüm satırları göster → 10 satır geri gelir.
- `null` → `No matches` / `Eşleşme yok` (NULL hiçbir zaman eşleşmez).
- Yazarken arayüz donmaz; sayaç bir an `0+` gibi titremez.

**Geçti:** Türkçe esnek eşleşmeler doğru; süzme ve geri alma doğru.

**Ön doğrulama:** `null_never_matches`, `filter_rows_builds_a_filtered_map`,
`a_sort_while_finding_rescans_in_the_order_shown`, `find_over_truncated_marks_incomplete`
(`results_find.rs`); `grid_find_filter_rows_then_copy_copies_the_kept_rows` (`cross_surface_m3c.rs`).

## 3. Satıra git (E1, E3)

**Yap:**

```sql
SELECT g AS n, 'Satır ' || g AS ad, md5(g::text) AS ozet FROM generate_series(1, 5000) g;
```

Izgarada ⌘L. Bölge biçiminiz Türkçe ise `1.000`, İngilizce ise `1,000` yazın, Enter. Sonra ⌘L →
`1500:3`, Enter. Sonra ⌘L → `9999`. Sonra ⌘L → `abc`.

**İyi görünüm:**
- Başlık `Go to row` / `Satıra git`; ipucu `row or row:column` / `satır ya da satır:sütun`.
- Yazarken canlı satır: `Row 1000, column …` / `1000. satır, … sütunu`; Enter → odak `n = 1000`
  satırında, satır görünür.
- `1500:3` → 1500. satır, `ozet` sütunu.
- `9999` → `There's no row 9999: the result has 5,000 rows.` / `9999. satır yok: sonuçta 5.000 satır
  var.` (sayı biçimi bölgeye göre).
- `abc` → `That isn't a row number.` / `Bu bir satır numarası değil.`

**Geçti:** doğru satır, Türkçe gruplama (`1.000` = bin) doğru okunuyor, hatalar anlaşılır.

**Ön doğrulama:** `goto_row_reads_locale_grouping`, `goto_out_of_range_names_the_last_row`
(`hacivat-ui-grid/tests/goto.rs`); `goto_past_loaded_fetches_with_a_confirmation_above_1m`
(`results_find.rs`); `go_to_row_1_000_in_turkish_reaches_view_row_1000_under_a_sort`
(`cross_surface_m3c.rs`).

## 4. Görünümler: Tablo, Devrik, Metin; Kayıt (E1, E3)

**Yap:** `SELECT * FROM demo_sehir;` Sonuç araç çubuğunun sağındaki **Table / Transposed / Text**
(**Tablo / Devrik / Metin**) seçicisinden **Transposed**'a, sonra **Text**'e, sonra **Table**'a geçin.
Bir satır seçip **Record** / **Kayıt** düğmesine basın; kayıt panelinde ▲▼ (ya da ⌥↑ / ⌥↓) ile gezin.
Devrik görünümdeyken ⌘F ve ⌘L'ye basın.

**İyi görünüm:**
- Devrik: her sütun bir satır, solda donmuş ad sütunu; sıralama oku yok.
- Metin: düz metin tablo (ham değerler); satır kopyalanabilir.
- Kayıt paneli: `Row 4 of 10` / `Satır 4/10`; ▲▼ satırı değiştirir.
- Devrik ve Metin'de bul ve satıra git **kapalıdır** (onaylı kural).
- Görünüm değişince odak gösterilen görünüme geçer; Escape ızgarada kalır.

**Geçti:** üç görünüm ve kayıt doğru; devrikte bul/git kapalı.

**Ön doğrulama:** `transposed_disabled_above_100k_rows`,
`escape_and_tab_switches_put_the_focus_on_the_view_shown` (`results_views.rs`);
`find_closes_on_a_tab_shown_transposed`, `goto_is_off_on_a_tab_shown_transposed`,
`transposed_view_rebuilds_at_a_rerun_while_it_streams` (`results_m3c_int.rs`).

## 5. Değer görüntüleyici: biçimli JSON, kırpılmış değer, "Re-run" ve okuma dışı ret (E1, E3)

**Yap (a):**

```sql
SELECT 1 AS id, repeat('ab', 1000000) AS uzun, '{"şehir":"İzmir","nüfus":4500000}'::jsonb AS j;
```

`j` hücresinde ⇧↵: **Pretty** / **Biçimli**, sonra **Raw** / **Ham**. Kapatın. `uzun` hücresinde ⇧↵.
Alt satırdaki kırpılma notunu okuyun, sonra **Re-run with a 64 MB value limit** / **64 MB değer
sınırıyla yeniden çalıştır**'a basın. Yeniden `uzun`'u açın. **Copy raw value** / **Ham değeri
kopyala**.

**Yap (b), okuma olmayan ifade:**

```sql
INSERT INTO demo_buyuk VALUES (1, repeat('ab', 1000000)) RETURNING v;
```

`v` hücresinde ⇧↵ → Re-run düğmesine basın. Sonra `SELECT count(*) FROM demo_buyuk;`.

**İyi görünüm:**
- Başlık `Value viewer` / `Değer görüntüleyici`; alt başlık `text · row 1` / `text · 1. satır`.
- Biçimli JSON yalnızca gösterimdir; Ham'a dönünce asıl metin.
- `uzun` kırpılmış: `Showing the first 1 MB of …` / `Değerin ilk 1 MB kısmı gösteriliyor (toplam …).`
- Re-run, olağan Çalıştır yolundan geçer (dev'de soru yok) ve aynı sorguyu tekrar çalıştırır; yeni
  sonuçta `uzun` kırpılmaz.
- (b): Re-run **çalışmaz**: `Only a query that reads data is run again with a larger value limit.
  Run this statement again from the editor.` / `Daha büyük bir değer sınırıyla yalnızca veri okuyan
  sorgular yeniden çalıştırılır. Bu ifadeyi düzenleyiciden yeniden çalıştırın.` `count(*)` ret
  öncesiyle aynı (satır ikinci kez eklenmedi).
- Pencere 960 genişliğe daraltılırsa kısa düzen: en az bir değer satırı, kırpılma notu ve Re-run
  klavyeyle (PgDn/Tab) erişilebilir (P2-05).

**Geçti:** okuyan sorgu yeniden çalışıyor ve tam değer geliyor; yazan sorgu hiçbir zaman tekrarlanmıyor.

**Ön doğrulama:** `value_viewer_pretty_json_is_display_only`,
`truncated_value_offers_a_rerun_with_the_raised_limit`, `rerun_with_raised_limit_goes_through_the_gate`,
`rerun_with_raised_limit_runs_only_a_read`, `rerun_with_raised_limit_refuses_a_write_on_production`
(`results_views.rs`); `value_viewer_rerun_after_a_sort_takes_runs_path_and_drops_the_sort`
(`cross_surface_m3c.rs`).

## 6. Geçmiş: arama ve geri çağırma (E1)

**Yap:** ⌘⇧H, arama alanına `izmir` yazın; bir satırı seçip **Insert** / **Ekle**. Sonra
düzenleyicide boş bir satırda ⌃⌥↑ üç kez, ⌃⌥↓ bir kez.

**İyi görünüm:**
- `izmir` → `İzmir` geçen ifadeler bulunur (Türkçe katlama).
- ⌃⌥↑: satıra geçmişteki ifade gelir, durum satırında `From history: statement 1 of n · …` /
  `Geçmişten: ifade 1/n · … daha eski · … daha yeni`. Geri çağırma hiçbir şey çalıştırmaz; tek ⌘Z
  geri alır.

**Geçti:** arama ve geri çağırma doğru, hiçbir şey kendiliğinden çalışmıyor.

**Ön doğrulama:** `history_insert_and_run_meets_the_gate` (`cross_surface_m3b.rs`);
`history_recall_at_the_last_line` (`hacivat-ui-editor/tests/lines.rs`).

## 7. Izgara bağlam menüsü (E3)

**Yap:** `SELECT * FROM demo_sehir;` `ad` sütununda bir hücreye sağ tıklayın; menüyü okuyun, Escape.
Satır numarasına sağ tıklayın → **Show as record** / **Kayıt olarak göster**. `nufus` başlığına sağ
tık → **Raw display** / **Ham görünüm**. Klavyeyle: bir hücreye gidip ⇧F10.

**İyi görünüm:**
- Hücre menüsü: `Copy`, `Copy with headers`, `Copy rows`, `Copy column name`, `Open value viewer`,
  `Sort ascending`, `Sort descending`, `Freeze up to here`, `Hide column`, `Fit width`,
  `Raw display`, `Go to row…`, `Show as record` (TR: `Kopyala`, `Başlıklarla kopyala`,
  `Satırları kopyala`, `Sütun adını kopyala`, `Değer görüntüleyiciyi aç`, `Artan sırala`,
  `Azalan sırala`, `Buraya kadar dondur`, `Sütunu gizle`, `Genişliği sığdır`, `Ham görünüm`,
  `Satıra git…`, `Kayıt olarak göster`).
- Kullanılamayan satırlar gri ve nedenli: örneğin sıralı değilken `Clear sort` → `Not sorted` /
  `Sıralı değil`; gizli sütun yokken `Show all columns` → `None hidden` / `Gizli sütun yok`.
- Ham görünüm: `5,800,000` → `5800000` yalnız o sütunda.
- ⇧F10 menüyü odak hücresinde açar; menü pencere dışına taşmaz.

**Geçti:** menü doğru sütuna/satıra uygulanıyor, nedenler anlaşılır, klavyeyle açılıyor.

**Ön doğrulama:** `menu_variants_and_disabled_reasons`, `raw_display_per_column_and_global`,
`a_menu_over_an_old_result_closes_and_never_acts_on_the_new_one`,
`show_as_record_shows_the_row_the_menu_is_over` (`results_menu.rs`).

## 8. Donmuş sütunlar ve çoklu dikdörtgen seçim (E3)

**Yap:**

```sql
SELECT g AS id, 'Satır ' || g AS ad,
       md5(g::text) AS c01, md5((g+1)::text) AS c02, md5((g+2)::text) AS c03, md5((g+3)::text) AS c04,
       md5((g+4)::text) AS c05, md5((g+5)::text) AS c06, md5((g+6)::text) AS c07, md5((g+7)::text) AS c08
FROM generate_series(1, 5000) g;
```

`ad` başlığına sağ tık → **Freeze up to here** / **Buraya kadar dondur**. Yana kaydırın. Sonra bir
blok seçin (sürükleyin), ⌘ basılıyken ikinci bir blok sürükleyin. Şeride bakın. Bir hücrede ⌥↑ iki
kez, ⌥↓ bir kez. ⌘C.

**İyi görünüm:**
- `id` ve `ad` yana kaydırırken yerinde kalır; ipucu `Frozen: stays in place when you scroll
  sideways` / `Donduruldu: yana kaydırınca yerinde kalır`.
- Bölme çok darsa dondurma reddedilir: `Bu sütunlar dondurulamaz: kaydıracak yer kalmaz…`.
- İki blok seçili; şerit `2 ranges · N cells` / `2 aralık · N hücre`.
- ⌥↑ seçimi büyütür (hücre → satır parçası → satır → tümü), ⌥↓ küçültür.

**Geçti:** donmuş sütunlar ve çoklu seçim doğru; kaydırma akıcı.

**Ön doğrulama:** `shift_extends_from_a_gutter_row_take_view_rows_under_a_shuffled_sort`,
`counting_10000_ranges_under_2ms` (`model_viewmap.rs`);
`the_strip_counts_the_ranges_of_a_multi_range_selection` (`results_view.rs`);
`deactivating_the_window_stops_the_autoscroll` (`element_frozen.rs`).

## 9. MySQL ve SQLite: aynı günlük iş (E1, üç motor)

**Yap:** `mysql84` konsolunda:

```sql
CREATE TABLE IF NOT EXISTS demo_sehir (id int PRIMARY KEY, ad varchar(64) NOT NULL, bolge varchar(64), nufus int);
DELETE FROM demo_sehir;
INSERT INTO demo_sehir VALUES (1,'Zonguldak','Karadeniz',588000),(2,'Ankara','İç Anadolu',5800000),
  (3,'Çorum','Karadeniz',525000),(4,'İzmir','Ege',4500000),(5,'Iğdır','Doğu Anadolu',203000);
SELECT * FROM demo_sehir;
```

`sqlite-demo` konsolunda aynı dört ifadeyi (`varchar(64)` yerine `text`) çalıştırın. Her ikisinde:
`ad` artan sırala (menüden), ⌘F `izmir`, ⌘L `3`, `ad` hücresinde ⇧↵, Devrik görünüm, ⌘⇧H'de `izmir`.

**İyi görünüm:** adım 1–6 ile aynı davranış; sıralama yine kod noktası ve aynı ipucu; Türkçe esnek
bul çalışır.

**Geçti:** üç motorda aynı günlük iş.

**Ön doğrulama:** ızgara özellikleri motordan bağımsız (`ResultSource` üstünde); motora özgü
karşılaştırıcılar `hacivat-values/tests/compare.rs` (MySQL sıfır tarihleri, SQLite int/float).

## 10. Çoklu imleç (E2)

**Yap:** Yeni bir `pg17` konsolu açın (komut paleti ⌘K) ve yazın:

```sql
SELECT 'a' AS x;
SELECT 'b' AS x;
SELECT 'c' AS x;
```

İmleci 1. satırda `SELECT`'in hemen sonrasına koyun. ⌥⌘↓ iki kez. ` /*m*/` yazın. ⌘Z. Sonra ⌘↵.
Sonra `x` sözcüğünü çift tıkla seçip ⌃G iki kez; Escape; yine `x` seçiliyken ⌃⌘G. Son olarak
Option basılıyken üç satır boyunca dikey sürükleyin (sütun seçimi).

**İyi görünüm:**
- Üç imleç; durum çubuğunda `3 carets` / `3 imleç`. Yazılan metin üç satıra birden girer.
- Tek ⌘Z üçünü birden geri alır.
- ⌘↵ yalnızca **ana imlecin** ifadesini çalıştırır (tek sonuç) (M3c-02).
- ⌃G bir sonraki `x`'i ekler (büyük/küçük harf ve Türkçe harfler tam eşleşir); ⌃⌘G hepsini seçer.
- Option-sürükleme sütun seçer; kısa satırda imleç satır sonuna konur (M3c-12).

**Geçti:** çoklu imleç, tek geri alma, yalnız ana imleç çalışır.

**Ön doğrulama:** `typing_at_10k_carets_under_50ms`, `select_next_occurrence_is_case_and_turkish_exact`
(`hacivat-text/tests/multi.rs`); `one_undo_removes_a_multi_caret_edit`, `run_follows_the_main_caret`
(`multi_view.rs`); `several_carets_run_only_the_main_carets_statement` (`exec_ui.rs`);
`multi_caret_typing_is_one_undo_and_run_runs_the_main_caret` (`cross_surface_m3c.rs`).

## 11. IME: M3a listesinin yeniden çalıştırılması + birden çok imleç (E2)

**Yap:** `docs/benchmarks/m3a-ime-checklist.md` içindeki macOS tablosunu bu derlemede yeniden
çalıştırın, sonra aynı dosyadaki **"M3c: several carets"** bölümündeki M1–M10 satırlarını. Gözlem
ve Geçti/Kaldı sütunlarını o dosyada doldurun.

**İyi görünüm (özet):** Japonca/Çince bileşim üç imlecin hepsine yazılır, aday penceresi yalnızca
ana imlecin altında; bir imleç dize içindeyse durum satırı `Input method: typing at the main caret
only, …` / `Giriş yöntemi: … yalnızca ana imleçte yazılıyor` der ve yalnız ana imlece yazılır;
bileşim sırasında ⌘↵ hiçbir şey çalıştırmaz; Türkçe Q harfleri her imleçte tam.

**Geçti:** listedeki satırlar geçti.

**Ön doğrulama:** `ime_mirrors_to_every_caret_in_plain_text`,
`ime_falls_back_to_the_primary_caret_inside_a_string` (`ime_mirror.rs`);
`a_platform_range_moves_every_mirror_by_the_same_distances` (`hacivat-text/tests/multi.rs`);
`crates/ui/hacivat-text/tests/ime_traces.rs`.

## 12. Katlama (E2)

**Yap:** Konsolda:

```sql
SELECT id,
       ad,
       bolge,
       nufus
FROM demo_sehir
WHERE nufus > 100000
ORDER BY nufus DESC;
```

İmleç bu ifadenin içindeyken **⌥⌘[** (katla). Katlanmış başlıkta imleç varken ⌘↵. Sonra **⌥⌘]**
(aç). Kenar çubuğundaki katlama okuna (chevron) tıklayarak bir kez daha katlayıp açın. Komut
paletinde (⌘K) `Fold all` / `Tümünü katla` ve `Unfold all` / `Tüm katlamaları aç`.

**Klavye notu:** Türkçe Q/F, Almanca, Fransızca, Lehçe düzenlerinde `[` ve `]` Option/AltGr ile
yazıldığı için ⌥⌘[ / ⌥⌘] bu düzenlerde basılamayabilir (bilinen kısıt). Basılamıyorsa paletten
`Fold` / `Katla` ve `Unfold` / `Katlamayı aç` kullanın ve sonuç tablosuna not edin (m3c.md 8.2,
soru 3). "Tümünü katla/aç" için kısayol yok (P2-08 bekliyor).

**İyi görünüm:**
- Başlık satırı kalır, altındaki satırlar gizlenir; `⋯` işareti ve `6 lines` / `6 satır`; ipucu
  `Lines 2–7 are folded. Click to unfold.` / `2–7. satırlar katlandı. Açmak için tıklayın.` (ifade
  konsolun ilk satırında değilse numaralar kayar).
- Katlanmış başlıkta ⌘↵ ifadenin **tamamını** çalıştırır.
- Açınca satırlar geri gelir; düzenleme kaybı yok.

**Geçti:** katlama/açma klavye ve fareyle çalışıyor; katlı ifade tam çalışıyor.

**Ön doğrulama:** `ui_edit_10_cases_1_to_5`, `line_at_y_on_50k_lines_with_1000_folds_under_5us`
(`hacivat-text/tests/fold.rs`); `folded_lines_are_not_in_a11y_runs`,
`exec_mark_inside_a_fold_paints_on_the_header` (`fold_view.rs`);
`run_at_a_caret_inside_a_fold_runs_the_whole_statement` (`cross_surface_m3c.rs`).

## 13. Düzenleyicide bul (Türkçe esnek), satır komutları, kenar ▶ (E2)

**Yap (bul):** Konsola yazın:

```sql
-- İzmir, IZMIR, izmir, Izmir
SELECT 'İzmir' AS a, 'ılık' AS b;
```

⌘F, `izmir` yazın; **Turkish-tolerant** / **Türkçe esnek** düğmesini kapatıp açın. ⌘G, ⇧⌘G.
Escape. `ılık` sözcüğünü seçip ⌘E (seçimi bulmada kullan), sonra ⌘G.

**Yap (satır komutları):** 2. satırda ⌘D (çoğalt), ⇧⌘K (sil), ⌥⇧↑ / ⌥⇧↓ (taşı), ⌘/ (yorum).

**Yap (▶):** Fareyi 2. satırın kenar çubuğuna getirin; ▶'nin ipucunu okuyun; ▶'ye tıklayın.

**İyi görünüm:**
- Türkçe esnek açıkken `izmir` dört biçimin hepsini bulur (`1 of 5` / `1/5`; yorum satırındaki 4 +
  dizedeki 1); kapalıyken noktalı `İ` ile yazılanlar bulunmaz. İpucu: `Turkish-tolerant: İ, I, ı and i find each
  other, and accents are ignored.` / `Türkçe esnek: İ, I, ı ve i birbirini bulur, aksanlar yok
  sayılır.`
- Escape bul çubuğunu kapatır; imleç yalnızca bulmanın seçimi duruyorsa geri döner.
- ⌘E sonrası ⌘G `ılık`'ı bulur.
- ⌘/ satırı `-- ` ile yorumlar ve geri alır; taşıma ve çoğaltma tek ⌘Z ile geri alınır.
- ▶ ipucu `Run this statement` / `Bu ifadeyi çalıştır`, `Last runs` / `Son çalıştırmalar`; tıklama
  ⌘↵ ile aynı yoldan çalıştırır.
- ▶ sütunu klavyede tek bir Tab durağıdır; ↑/↓ gezer, Escape düzenleyiciye döner (M3c-04).

**Geçti:** bul, satır komutları ve ▶ doğru.

**Ön doğrulama:** `turkish_tolerant_matches_dotted_and_dotless_i`,
`exact_mode_does_not_fold_turkish_letters`, `use_selection_for_find_seeds_the_closed_bar`
(`find_bar.rs`); `toggle_comment_uses_the_dialect_line_comment`, `move_line_moves_a_fold_with_it`
(`lines.rs`); `gutter_play_runs_the_rows_statement`, `each_gutter_column_is_one_tab_stop_walked_with_the_arrows`
(`gutter_m3b.rs`); `find_select_all_then_type_edits_every_match_as_one_undo` (`cross_surface_m3c.rs`).

## 14. Güven sinyalleri 120 px bölmede, Türkçe (E4)

**Yap:**
1. Başlık çubuğundaki EN/TR ile **Türkçeye** geçin.
2. `pg17-prod`'a bağlanın (kip `Manuel`). ⌘K → `Sağa böl`. Ortadaki ayırıcıyı sağa, durana kadar
   sürükleyin: sağ bölme 120 px'e iner (en dar sınır). Sağ bölmede `pg17-prod` konsolu açık olsun.
3. Sağ bölmede çalıştırın:

   ```sql
   UPDATE demo_orders SET total = total WHERE id = 2;
   ```

   → `Koru`. Rozete bakın. Sonra rozetteki Rollback düğmesine basın.
4. `SELECT pg_sleep(5);` → çalışırken bakın; ⌘. ile durdurun.
5. Sağ bölmede `pg17-ro`'ya bağlı bir konsol açın (ya da sekmesine geçin).

**İyi görünüm (hiçbiri kesilmez, kırpılmaz, üst üste binmez):**
- Ortam: üretim rengi şerit/örnek her zaman görünür; bağlantı adı uzunsa kısaltılabilir ama renk
  örneği kalır.
- Kip çipi sözcüğünü korur: **`Manuel`** (gerekirse çip içinde alt satıra geçer).
- Çalıştır ve Durdur yalnız simge; adları ipucunda (`Run · ⌘↵`).
- Rozet: `Transaction · 1 ifade` (süre düşer), yanında **Commit et** ve **Rollback et** simge
  düğmeleri bölmenin içinde.
- Çalışırken: amber işaret + saat (ekranda "Çalışıyor" sözcüğü yok, ipucunda var). Durdururken
  `İptal ediliyor…` bütün.
- Salt okunur: kilit + `salt okunur` sözcüğü bütün (üretimde kilit kırmızı).
- Durum çubuğundaki güven hücreleri yer vermez.

**Kısa dar bölme kontrolü:** Ayırıcıyı 220 px civarına getirin, `SELECT * FROM demo_sehir;`
çalıştırın. Sonuç araç çubuğunda önce görünüm seçici ve Kayıt düğmesi küçülür; **Kopyala** en son
gizlenir (M3c-18). Şeritteki metinler kesilmez.

**Bilinen (güven sinyali değil):** `m3c-narrow-fixes` birleşmeden önce, 120 px'te Azaltılmış kip
afişinin gövdesi kesilebilir (F11) ve boş konsol yer tutucusu/sekme adı sahte dillerde kesilir
(F10, F12). Görürseniz not edin; kapıdan önce düzeltilmiş olmaları gerekir.

**Geçti:** 120 px'te her güven sinyali bütün ve okunur.

**Ön doğrulama:** `trust_items_whole_down_to_120px_in_every_locale`,
`the_strips_cursor_open_stays_whole_in_a_narrow_pane`, `copy_gives_way_last_in_a_narrow_result_toolbar`,
`the_narrow_lock_is_red_on_production` (`toolbar_narrow.rs`);
`a_production_console_in_a_120px_pane_runs_with_its_trust_signals_whole` (`cross_surface_m3c.rs`,
Türkçe, üretim, 120 px). Görüntüler `narrow-*` (12 durum × 16).

## 15. EN ↔ TR: yeni yüzeyler

**Yap:** Türkçedeyken adım 2, 3, 4, 5, 7, 12 ve 13'ün yüzeylerine kısaca bakın. Satıra git'te
`1.000` yazın. Pencereyi 960 genişliğe daraltıp değer görüntüleyiciyi ve bul çubuğunu bir kez daha
açın.

**İyi görünüm:** her metin Türkçe ve doğal; "transaction", "commit", "rollback" İngilizce; kesik,
taşan etiket yok; `1.000` → 1000. satır. Terimler: `Türkçe esnek`, `Devrik`, `Biçimli/Ham`,
`Satırları süz`, `harmanlama` (M3c-19, onaylı).

**Geçti:** Türkçe metinler doğal ve tutarlı; önerileri not edin.

**Ön doğrulama:** `cargo xtask l10n check` 0 hata; 4.860 görüntü (en, en-XA, tr, tr-XT × koyu/açık
× 1280/960); TR metinlerin hepsi `# Review: PO` işaretli.

## 16. B1 ve B3 ölçümleri (E5)

Demo değil, ölçümdür; uygulamayı kapatıp yapın. Komutlar, eşikler ve sonuç tablosu:
`docs/benchmarks/m3c-bench-notes.md` bölüm 2–5 (T44). Özet:

- Ekran uyanık, oturum açık; diğer derlemeler ve ajanlar kapalı; bench penceresi önde; fare ve
  klavyeye dokunmayın. Ekranın uyumaması için komutları `caffeinate -dis` ile çalıştırın.
- Önce eski dökümleri kenara alın (`target/bench/`).
- B3: `b3 --bench typing`, `typing-with-results`, `typing-multi-caret`, `typing-with-folds`,
  `typing-long-line`, `typing-19mb` (+ `open`, `scroll`, `ime-bounds`). Ekran ≥ 118 Hz, biçim `tr-TR`.
- B1-real: `HACIVAT_DOCKER_SET=pr $E/b1_real` (1440 × 900, yerleşik ekran 2×, ≥ 118 Hz) ve
  `--size 2560x1440` (1× harici ekran ana ekran olarak), ayrıca `--sorted`. `pg17` 1M × 50 verisi
  `generate_series` ile sunucuda üretilir; hiçbir şey kurulmaz, kapsayıcı yeniden oluşturulmaz.
- Her çalıştırma `RESULT: PASS|FAIL|INVALID|n/a` ile biter. INVALID "ölçülemedi" demektir, FAIL değil.
- Sonra `cargo xtask bench-report --milestone m3c …` (komut bench notlarında); `host` sütununu
  "PO's Mac" yapın.

## 17. VoiceOver (isteğe bağlı, kapıyı engellemez; PO: "Sonra")

**Yap:** ⌘F5 ile VoiceOver'ı açın. `SELECT * FROM demo_sehir;` sonucunda:

1. Tab ile ızgaraya girin; oklar hücreyi okur, satır değişince `Row n of N`.
2. VO tablo gezinmesi (VO+oklar) sentetik hücrelerde çalışır.
3. VO imleci odak hücresini izler (active descendant).
4. Canlı bölge bir kez okunur; sekme değiştirip dönünce yeniden okunmaz.
5. ⇧+ok seçimde `N cells selected`.
6. Ortadaki bir hücreden Tab ızgaradan çıkar; ⇧Tab geri girer.
7. Hücrede VO eylemleri: Show menu (bağlam menüsü açılır), Scroll to visible.
8. Başlık hücreleri sıralama durumunu okur.
9. ↓ basılı tutunca tek bir `Row n of N`; satır içinde fazladan okuma yok.

Ayrıca: kenar ▶ ve katlama düğmeleri ile değer görüntüleyici/kayıt paneli okunuyor mu?

**Geçti/Kaldı:** sonucu yazın. Kötü sonuç bir çatal yamasına (R-M3-69) karar verdirir; L4 M9'da kapılar.

---

## Sonuçlar

| Adım | Geçti / Kaldı | Not |
|---|---|---|
| 1 Sıralama, ipucu, kopya | | |
| 2 Izgarada bul, süz | | |
| 3 Satıra git | | |
| 4 Görünümler, kayıt | | |
| 5 Değer görüntüleyici, Re-run, ret | | |
| 6 Geçmiş, geri çağırma | | |
| 7 Bağlam menüsü | | |
| 8 Donmuş sütunlar, çoklu seçim | | |
| 9 MySQL ve SQLite | | |
| 10 Çoklu imleç | | |
| 11 IME listesi | | |
| 12 Katlama | | |
| 13 Bul, satır komutları, ▶ | | |
| 14 Güven sinyalleri 120 px TR | | |
| 15 EN ↔ TR | | |
| 16 B1/B3 ölçümleri | | (m3c.md bölüm 6) |
| 17 VoiceOver (isteğe bağlı) | | |

Bittiğinde: demo tablolarını isterseniz dev profilinden silin
(`DROP TABLE demo_sehir; DROP TABLE demo_buyuk;`; MySQL ve SQLite'ta da `DROP TABLE demo_sehir;`).
`demo_orders` M3b demosundan kalmışsa bırakılabilir.
