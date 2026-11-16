# M3a PO demosu: "Sorgu çalıştır" kontrol listesi

Bu liste M3a kapısının (Görev 37) yalnızca PO'nun yapabileceği kısmıdır. 12 adım gerçek bir Mac'te,
gerçek Docker sunucularına karşı, gerçek pencere, gerçek klavye ve gerçek VoiceOver ile elle
çalıştırılır. Her adımda dört şey vardır: ne yapılacağı (tam metin ve tuşlarla), nereye bakılacağı,
beklenen metin (EN ve TR) ve "geçti"nin ne demek olduğu. Her adımın sonunda, kapı ajanının o adımı
hangi otomatik testlerle önceden doğruladığı yazılıdır.

Önceden doğrulama adımın yerini tutmaz. Uygulama testleri sahte (fake) sürücü ve başsız (headless)
GPUI ile çalışır; Docker testleri arayüzsüz çalışır. Kapı ajanı arayüzü elle süremez: gerçek pencere,
gerçek IME, gerçek pano ve gerçek ekran okuyucu yalnızca bu listeyle görülür.

Kapı kaydı: `docs/benchmarks/m3a.md` (İngilizce). Önceden doğrulama komutları ve sonuçları oradaki
bölüm 2'dedir (yerel kapı) ve bölüm 3'tedir (CI).

## 0. Hazırlık

1. **Docker yığını** (`hacivat-m2a`) ayakta bırakıldı. Kontrol (yalnızca okur):

   ```
   cargo xtask docker status --set full
   ```

   İyi görünüm: 19 `m2a-*` kapsayıcı `healthy`. Yığın kapalıysa depo kökünden
   `cargo xtask docker up --set full`.

2. **Temiz ilk açılış.** Eski uygulama klasörlerini kenara taşıyın (klasör yoksa komut hata verir,
   sorun değil):

   ```
   mv "$HOME/Library/Application Support/dev.hacivat.Hacivat" "$HOME/Library/Application Support/dev.hacivat.Hacivat.m2b-bak"
   mv "$HOME/Library/Caches/dev.hacivat.Hacivat" "$HOME/Library/Caches/dev.hacivat.Hacivat.m2b-bak"
   ```

3. **Uygulama** (release derleme; adım 10 dışında İngilizce başlar):

   ```
   cargo run --release -p hacivat -- --lang en
   ```

4. **Test sunucuları** (yalnızca demo içindir, gerçek parola değildir; `docker/m2a/README.md` ve
   `docker/m2a/targets.toml`): kullanıcı `hv_test`, parola `hv_test_pw`, veritabanı `hv_test`.
   Portlar (`127.0.0.1`): `pg17` 54317, `mysql84` 33084.

5. **Üç bağlantı profili** (⌘N, "Save and connect"):
   - `pg17`: PostgreSQL, Host `127.0.0.1`, Port `54317`, kullanıcı/parola/veritabanı yukarıdaki gibi,
     Ortam (Environment / **Ortam**) `dev` / `geliştirme`.
   - `pg17-prod`: aynısı, Ortam **`production`** / **`üretim`** (adım 7).
   - `mysql84`: MySQL, Port `33084`, aynı kimlik bilgileri, Ortam `dev` (adım 9).
   - SQLite: yeni bir dosya, örneğin `~/hv-demo-m3a.sqlite` (adım 9).

6. Hatırlatmalar:
   - Bağlı olmayan bir konsol, M2'deki bağlantı ana sayfasını gösterir (R-M3-37): editör, konsol bir
     bağlantıya bağlandığında görünür.
   - Run (Çalıştır) kısayolu ⌘↵; durdurma ⌘. (Escape hiçbir zaman iptal etmez, R-M3-38).
   - Bir şey farklıysa adım numarasıyla not alın; sonuçları sondaki tabloya işleyin. Uygulama içindeki
     İngilizce/Türkçe metinler bu belgede `EN` / `TR` olarak verilmiştir.

---

## 1. Açılış, bağlı olmayan konsol, `pg17`'ye bağlanma

**Yap:** Uygulamayı açın. Gezginde `pg17` satırına çift tıklayın.

**İyi görünüm:**
- Bağlanmadan önce konsol M2 ana sayfasını gösterir: **No query open** / **Açık sorgu yok** ve
  "Pick a connection in the Explorer, or add one." / "Gezgin'den bir bağlantı seçin ya da yeni bir
  tane ekleyin."
- Bağlanınca konsol bağlanır: sekme başlığı profil adıdır (`pg17`; aynı bağlantının ikinci konsolu
  `pg17 (2)`), başlık **eş aralıklı (mono)** yazıyla çizilir; editör görünür ve odaklanır.

- F5 (PO bulgusu 1): odaktaki konsol başka bir bağlantıya bağlıyken Gezgin'den (çift tıklama ya da
  **Connect** / **Bağlan**) bağlanılan profil için açık konsol yoksa, ona bağlı yeni bir konsol açılır
  ve odaklanır; o profilin konsolu zaten varsa o konsol öne gelir ve odaklanır. Bağlı olmayan odaktaki
  konsol yine önce bağlanır (R-M3-37).

**Geçti:** ana sayfa → bağlı editör geçişi, başlık mono.

**Ön doğrulama:** `unbound_console_shows_the_m2_home_and_binds_on_first_connect` (console.rs; başlık
"orders-dev" olur), `a_profile_that_appears_connected_binds_nothing` (console.rs),
`unbound_console_renders_the_m2_home`, `unbound_console_without_connections_keeps_the_m1_empty_state`,
`the_console_binding_hook_is_installed_and_opens_unbound` (m3a_routing.rs); görüntüler `console-empty`,
`console-editor`. **Otomatik kapsam yok:** başlığın mono yazıyla çizildiği (yalnızca bağlantı iletişim
kutusu için `machine_text_is_mono_and_secrets_and_placeholders_are_not` var) — gözle bakın.

## 2. Türkçe tanımlayıcılarla üç ifade; vurgu ve önizleme şeridi; seçici (chooser)

**Yap:** `pg17` konsoluna yazın:

```sql
CREATE TABLE IF NOT EXISTS müşteri (id int, şehir text, tutar numeric);
INSERT INTO müşteri VALUES (1, 'İzmir', 1234.56), (2, NULL, 7), (3, 'Şanlıurfa', 1234567.5);
SELECT id, şehir AS "müşteri şehri", tutar FROM müşteri ORDER BY id;
```

İmleci satırlar arasında gezdirin. Sonra imleci **ortadaki** ifadenin içine koyup ⌘↵'ye basın. Seçicide
oklarla gezinin, fareyle satırların üzerine gelin, Enter'a basın.

**İyi görünüm:**
- Anahtar sözcükler, dizgeler ve sayılar vurgulanır; Türkçe harfler tanımlayıcının parçası kalır.
- Önizleme şeridi (preview band) imleci izler: imlecin olduğu ifadeyi gösterir.
- ⌘↵ birden çok aday varsa seçiciyi açar: başlık **Choose what to run** / **Neyin çalışacağını
  seçin**; satırlar **Statement** / **İfade**, **Largest statement** / **En geniş ifade**,
  **From caret** / **İmleçten sona**, **Whole script** / **Tüm betik**. Fareyle üzerine gelinen
  satırın aralığı editörde önizlenir. Enter seçili olanı çalıştırır. Tek aday varsa seçici açılmaz.
- Escape seçiciyi kapatır, hiçbir şey göndermez; odak editöre döner.

**Geçti:** vurgu doğru, şerit imleci izliyor, seçici açılıyor, üzerine gelmek aralığı gösteriyor,
Enter yalnızca ortadaki ifadeyi çalıştırıyor (ilk çalıştırmada tablo yoksa ilk ifadeyi önce çalıştırın).

**Ön doğrulama:** `ask_opens_the_chooser_hover_previews_and_enter_runs_the_preselected`,
`one_candidate_runs_without_a_chooser`, `escape_closes_the_chooser_and_sends_nothing`,
`a_modified_enter_never_chooses`, `chooser_remembers_the_kind_per_console`,
`closing_the_chooser_gives_the_focus_back_to_the_editor`, `preview_and_run_agree_on_generated_scripts`
(Türkçe parçalarla; exec_ui.rs); `the_chooser_previews_on_hover_and_runs_the_highlighted_statement`
(cross_surface_m3a.rs); `a_caret_move_hides_the_old_band_until_the_new_answer`,
`the_chooser_override_replaces_the_band` (hacivat-ui-editor preview.rs); `ui_edit_11_cases_1_and_3`
(hacivat-sql-editor highlight.rs); E4 altınları ve önizleme/çalıştırma uyuşma özelliği
(`caret_sel_choose_goldens`, `preview_equals_execution_for_exact_items`). Görüntüler `preview-*`,
`chooser-candidates`, `chooser-hover`, `chooser-uncertain`.

## 3. İki ifadeyi seçip çalıştırma: güncelleme sayısı Çıktı'da, satırlar sonuç sekmesinde

**Yap:** INSERT ile SELECT'i birlikte seçin, ⌘↵.

**İyi görünüm:**
- Yalnızca satır döndüren ifade bir sonuç sekmesi açar. INSERT'in sonucu bir güncelleme
  sayısıdır: **Output** / **Çıktı** sekmesinde bir satır olarak görünür (**3 rows affected** /
  **3 satır etkilendi**, süresiyle). SELECT **Result 1** / **Sonuç 1** sekmesini açar (Output önde).
  (F5 düzeltmesi: önceki metin "Result 1 + Result 2" bekliyordu; INSERT sonuç sekmesi açmaz.)
- Her ifadenin satırında oluk işareti: INSERT için **3 rows affected · … ms** / **3 satır
  etkilendi · … ms**, SELECT için **executed · … ms · … rows** / **çalıştı · … ms · … satır**.

**Geçti:** Çıktı'da güncelleme sayısı, bir sonuç sekmesi, iki işaret; süre ve satır sayısı doğru.

**Ön doğrulama:** `selection_of_two_statements_runs_two_units` (exec_ui.rs),
`two_units_two_tabs_titled_result_n` (hacivat-exec service.rs), `titles_are_result_n`,
`output_leads_and_the_first_new_result_becomes_active` (results_model.rs), `ui_edit_19_cases_1_to_5`
(hacivat-ui-editor marks.rs; "executed · 34 ms · 12 rows" çizer). Görüntüler `marks-run`,
`marks-two-on-row`, `results-two`, `preview-selection`.

## 4. Bir milyon satır, açık imleç, kaydırınca getirme, imleci kapatma

**Yap:** Çalıştırın:

```sql
SELECT * FROM generate_series(1, 1000000);
```

Izgarayı aşağı kaydırın. Şeritte **Close cursor** / **İmleci kapat**'a basın. Sonra imleç açıkken bir
kez daha aynı SELECT'i çalıştırın ve bu kez kapatmadan `SELECT 1;` çalıştırın.

**İyi görünüm:**
- Satırlar parça parça gelir; şerit **Cursor open — holding a database snapshot** / **İmleç açık —
  veritabanı anlık görüntüsü tutuluyor**; durum çubuğunda **Cursor open** / **İmleç açık**.
- Kaydırma daha fazla satır getirir (**Fetching… N rows** / **Getiriliyor… N satır**).
- Close cursor sonrası: **Cursor closed after N rows.** / **N satırdan sonra imleç kapatıldı.**
- `SELECT 1` önce imleci bırakır, sonra çalışır. Yeni çalıştırmanın ilk sonucu aynı **Result 1** /
  **Sonuç 1** sekmesini alır (sekmeler ifade sırasına göre anahtarlanır), bu yüzden eski sonucun
  şeridi artık görünmez. Bırakma notu **Output** / **Çıktı** sekmesindedir (okunmamış noktası
  yanar); tek satırdır: **Cursor closed after N rows: another statement needed the session.
  Hacivat's transaction was rolled back.** / **N satırdan sonra imleç kapandı: oturum başka bir
  ifade için gerekiyordu. Hacivat'ın işlemi geri alındı.** (P26). (F5 düzeltmesi: önceki metin
  bunu şeritte bekliyordu; Çıktı sekmesine bakın.)

**Geçti:** satırlar korunuyor, metinler doğru, `SELECT 1` takılmadan çalışıyor.

**Ön doğrulama:** Docker `select_streams_suspends_fetches_more_and_closes` (hacivat-exec
docker_m3a_matrix.rs), `pg_suspended_million_rows_released_by_select_1` (hacivat-session
docker_m3a_cursor.rs); `another_statement_releases_the_cursor_first`,
`every_request_kind_releases_the_cursor`, `queued_fetch_mores_coalesce` (hacivat-session cursor.rs);
`a_suspended_cursor_says_so_and_its_release_rolls_back` (hacivat-exec service.rs);
`cursor_open_segment_closes_the_cursor` (status_follow.rs); `suspended_strip_buttons_reach_the_source`,
`strip_text_per_completeness` (results_view.rs). Görüntüler `strip-streaming`, `strip-suspended`,
`strip-closed-session-needed`, `status-cursor-open`.

## 5. Uzun bir sorguyu ⌘. ile 1 saniye içinde durdurma

**Yap:** Çalıştırın ve satırlar akarken ⌘.'ya basın:

```sql
SELECT g, pg_sleep(0.001) FROM generate_series(1, 200000) g;
```

Ardından tek başına `SELECT pg_sleep(30);` çalıştırıp ⌘.'ya basın.

**İyi görünüm:**
- Araç çubuğunda **Stop** / **Durdur**, basınca **Cancelling…** / **İptal ediliyor…**.
- 1 saniye içinde durur. Akış sonucu: **Cancelled after N rows** / **N satırdan sonra iptal
  edildi**; o ana kadarki satırlar ızgarada kalır.
- `pg_sleep` için oluk: **cancelled after …** / **… sonra iptal edildi**.

**Geçti:** ≤ 1 s, satırlar kalıyor, sonraki ifade normal çalışıyor.

**Ön doğrulama:** Docker `cancel_within_one_second_on_every_engine`,
`stop_fetching_keeps_rows_and_frees_the_session` (docker_m3a_matrix.rs);
`cancel_reaches_the_session_and_stop_fetching_keeps_rows` (exec_ui.rs, macOS'ta "cmd-." basar);
`stopping_a_streaming_result_keeps_its_rows_and_says_cancelled` (cross_surface_m3a.rs);
`cancel_resolves_running_and_drops_queue` (hacivat-session cancel.rs). Görüntüler `toolbar-running`,
`toolbar-cancelling`, `marks-cancelled`, `marks-stopped`, `strip-cancelled`.

## 6. 3 × 5 bir bloğu kopyalayıp metin düzenleyiciye yapıştırma

**Yap:** Adım 2'nin SELECT'ini, sütun sayısı 5 olacak biçimde çalıştırın:

```sql
SELECT id, şehir, tutar, NULL AS boş, 'a"b' AS tırnak FROM müşteri ORDER BY id;
```

İlk hücreden son hücreye kadar 3 × 5 bölgeyi seçin, ⌘C. TextEdit'e (düz metin) yapıştırın.

**İyi görünüm:**
- Bildirim: **Copied 3 × 5 cells** / **3 × 5 hücre kopyalandı**, alt satır **Tab-separated, raw
  values** / **Sekmeyle ayrılmış, ham değerler**.
- Yapıştırılan metin ham değerlerdir: sayılar gruplanmamış ve yerelden bağımsız (`1234567.5`,
  `1.234.567,5` değil), NULL **boş**, sütunlar sekmeyle ayrılmış.

**Geçti:** metin ham, NULL boş, satır/sütun sayısı doğru.

**Ön doğrulama:** `run_rows_then_copy_puts_raw_tsv_on_the_clipboard` (cross_surface_m3a.rs; beklenen
`1\tİzmir\n2\t\n1234567\tŞanlıurfa\n`), `copy_region_with_null_tab_and_quote_is_exact_tsv`,
`the_grid_shows_display_text_and_copy_takes_raw_text` (results_model.rs),
`raw_text_is_locale_free_under_turkish_process_locale` (hacivat-values valfmt_raw.rs). Görüntüler
`copy-copied`, `copy-copying`, `copy-too-much`, `grid-range`. **Otomatik kapsam yok:** başka bir
uygulamaya yapıştırma.

## 7. Üretim (production) profili: ince kırmızı çizgi, renk örneği, salt okunur uyarısı

**Yap:** `pg17-prod`'a bağlanın. Konsolda çalıştırın: `INSERT INTO müşteri VALUES (9, 'x', 1);`.
Çıktı günlüğüne (**Output** / **Çıktı**) bakın.

**İyi görünüm:**
- Editör bölmesinin üstünde tek bir kırmızı ince çizgi (hairline); araç çubuğunda ortam renk örneği
  (swatch) ve **Production is read-only in this version** / **Üretim bu sürümde salt okunur**
  (dar bölmede **read-only** / **salt okunur**). Kapsül ve durum çubuğu üretimi gösterir.
- INSERT gönderilmez (istemci katmanı, F5): oluk **not sent: read-only** / **gönderilmedi: salt
  okunur**; günlükte **Not sent: this console is read-only, and the statement can change data or
  schema.** / **Gönderilmedi: bu konsol salt okunur ve ifade veriyi ya da şemayı
  değiştirebilir.** Çıktı'da `25006` (sunucunun reddi) görünmemelidir: görünürse istemci katmanı
  çalışmamıştır. İlk anahtar sözcüğü yazmayı göstermeyen bir ifade (örneğin `WITH … INSERT`)
  sunucunun salt okunur işlemine takılır; bu ikinci katmandır.
- `SELECT` aynı konsolda çalışır.

**Geçti:** üç işaret de görünüyor, INSERT reddedilip günlüğe yazılıyor, tablo değişmiyor.

**Ön doğrulama:** Docker `production_console_rejects_insert_on_every_engine`,
`read_only_bypass_rows_blocked_through_run_without_classifier` (docker_m3a_matrix.rs);
`production_console_shows_the_notice_and_the_hairline` (console.rs),
`hairline_is_one_red_quad_at_the_pane_top` (m3a_routing.rs),
`a_production_insert_is_refused_logged_and_marked_failed` (cross_surface_m3a.rs),
`read_only_denial_marks_the_gutter_failed_with_its_reason` (exec_ui.rs),
`focusing_a_production_or_staging_console_swaps_the_capsule` (status_follow.rs),
`trust_items_stay_whole_in_a_narrow_pane_in_english` / `_in_turkish` (toolbar_narrow.rs). Görüntüler
`console-production`, `toolbar-read-only`, `toolbar-production-waiting`, `marks-denied`,
`status-production-result`.

## 8. Açık işlemle sekmeyi kapatma

**Yap:** `pg17` (dev) konsolunda çalıştırın:

```sql
BEGIN;
UPDATE müşteri SET tutar = tutar + 1 WHERE id = 1;
```

Konsol sekmesini kapatın (⌘W ya da sekmedeki ×). Önce Enter'a basın (konsol kalmalı), sonra yeniden
kapatıp **Roll back and close** / **Geri al ve kapat**'a basın.

**İyi görünüm:**
- Durum çubuğunda **Transaction open · 2 statements** / **İşlem açık · 2 ifade**.
- Soru: **pg17 has an open transaction with 2 statements** / **pg17 üzerinde 2 ifadelik açık bir işlem
  var**; gövde "Closing this console ends its transaction. …" / "Bu konsolu kapatmak işlemini
  sonlandırır. …". Düğmeler **Cancel** / **Vazgeç** (odakta), **Commit and close** / **Commit et ve
  kapat**, **Roll back and close** / **Geri al ve kapat**.
- Enter odaktaki Vazgeç'i seçer: konsol açık kalır. Geri al: konsol kapanır; yeni bir konsolda
  `SELECT tutar FROM müşteri WHERE id = 1` eski değeri verir.

**Geçti:** soru çıkıyor, Vazgeç odakta, geri alma kapatıyor ve değişikliği atıyor.

**Ön doğrulama:** `closing_a_console_in_a_transaction_asks_and_roll_back_closes_it`
(cross_surface_m3a.rs; ROLLBACK'in gönderildiğini doğrular), `closing_a_console_with_an_open_transaction_asks`
(Vazgeç odakta, Enter konsolu tutar), `quit_with_an_open_transaction_asks_and_cancel_keeps_the_app`,
`close_prompt_on_a_failed_transaction_offers_roll_back_only` (P16; exec_ui.rs),
`transaction_segment_only_for_user_transactions`, `failed_transaction_segment_says_so`
(status_follow.rs). Görüntüler `prompt-close`, `prompt-quit`, `prompt-failed`, `status-tx-open`.

## 9. MySQL 8.4 ve SQLite; birden çok sonuç kümesi döndüren `CALL`

**Yap:** `mysql84`'e bağlanın; adım 2'deki üç ifadeyi çalıştırın (MySQL'de de aynı akış). Sonra:

```sql
CALL hv_test.hv_t14_three();
```

`CALL`'u **tek başına** çalıştırın: editörde yalnızca bu satır olsun ya da imleci `CALL` satırına
koyup seçicide **Statement** / **İfade**'yi seçin. (Bu yordam Docker testlerinin kurduğu fikstürdür
ve `mysql84`'te vardır. Yoksa önce, ayrı bir çalıştırmada yalnızca
`CREATE PROCEDURE hv_test.hv_t14_three() BEGIN SELECT 1 AS n; SELECT 2 AS n; SELECT 3 AS n; END`
çalıştırın; bölücü `BEGIN … END` gövdesini tek ifade tutar.) F5 notu: `CREATE PROCEDURE` ile `CALL`
birlikte (tüm betik) çalıştırılırsa ve yordam zaten varsa, `CREATE` 42000 "already exists" ile
başarısız olur ve çalıştırma ilk hatada durur (R-M3-26): `CALL` hiç gönderilmez, oluğu **skipped: an
earlier statement failed** / **atlandı** der ve sonuç görünmez. PO demosundaki "CALL sonuç
vermedi" gözlemi büyük olasılıkla budur. Ardından SQLite bağlantısında adım 2'nin üç ifadesini
çalıştırın.

**İyi görünüm:**
- Seçici, şerit, oluk ve ızgara PostgreSQL'deki gibi.
- `CALL`: yalnızca ilk küme görünür (`n = 1`); şerit **2 more result sets not shown** / **2 sonuç
  kümesi daha gösterilmedi**; günlük **2 more result sets were read and not shown; only the first one
  is kept.** / **2 sonuç kümesi daha okundu ve gösterilmedi; yalnızca ilki tutulur.**

**Geçti:** iki motorda da aynı akış; CALL ilk kümeyi ve notu gösteriyor.

**Ön doğrulama:** Docker `typed_golden_tables_per_engine_and_version` (PG 14–18, MySQL 8.0/8.4,
MariaDB 10.6/10.11/11.4, SQLite), `select_streams_suspends_fetches_more_and_closes`
(docker_m3a_matrix.rs), `call_with_three_sets_keeps_the_first_and_counts_two` (hacivat-driver-mysql
typed_wire.rs ve docker_typed.rs), `later_result_sets_are_counted_and_noted` (hacivat-session run.rs),
`more_result_sets_are_logged` (hacivat-exec service.rs), `the_strip_renders_its_sentence_and_more_result_sets`
(results_view.rs). Görüntüler `console-mysql-comment`, `strip-more-result-sets`.

## 10. EN ↔ TR: yeni yüzeylerde İngilizce kalıntı yok; sayılar bölgeyi izler

**Yap:** ⌘, → **Language** / **Dil**: Türkçe. Adım 2–9'un yüzeylerini (seçici, şerit, oluk, ızgara,
günlük, işlem sorusu, üretim uyarısı, durum çubuğu) yeniden açın. Sonra **Regional format** /
**Bölgesel biçim**: Türkiye; adım 2'nin SELECT'ini yeniden çalıştırın. Sonra yeniden English.

**İyi görünüm:**
- Türkçede hiçbir yeni yüzeyde İngilizce kalıntı yok (veri ve SQL metni hariç; sunucu metni kendi
  alanında aynen kalır).
- Bölge Türkiye iken ızgarada `tutar`: `1.234,56` ve `1.234.567,5`; `id` gibi kimlik sütunları
  gruplanmaz; tarihler ISO kalır. Kopyalama yine ham (`1234.56`).
- Dil değişince açık sekmeler ve yeni sekmeler aynı biçimi izler.

**Geçti:** kalıntı yok, sayılar bölgeyi izliyor, kopyalama ham.

**Ön doğrulama:** görüntü matrisi her M3a yüzeyini `en`, `tr`, `en-XA`, `tr-XT` ile çizer; sözde
yerellerde köşeli parantezsiz tek bir Fluent etiketi kareyi düşürür (kontrol (a)); kapı çizimi 0 ihlal
verdi (`m3a.md` bölüm 2). `new_display_settings_format_again`,
`the_consoles_display_reaches_its_tabs_old_and_new` (results_model.rs), `decimal_tr_and_en_and_raw`
(hacivat-values valfmt_display.rs), `every_log_id_exists_in_en_and_tr`,
`copy_invariant_is_iso_and_ascii_under_turkish` (hacivat-exec log.rs),
`at_960_in_turkish_the_trust_segments_keep_their_place` (status_follow.rs); `cargo xtask l10n check`.
**Otomatik kapsam yok:** tek bir testte bütün yüzeylerde canlı dil değişimi.

## 11. Çıkıp yeniden açma: konsol metni geri gelir

**Yap:** `pg17` konsoluna ayırt edici bir metin yazın (örneğin `-- demo 11 ğüşıöçİ`), 2 saniye bekleyin,
⌘Q (açık işlem yoksa soru çıkmaz). Uygulamayı aynı komutla yeniden açın.

**İyi görünüm:** konsol sekmesi geri gelir, metin aynen yerindedir (Türkçe harfler dahil). Başarılı geri
yükleme hiçbir bildirim göstermez. Bir sorun olursa: **Couldn't reopen …** / **… yeniden açılamadı**
ve dosya diskte değiştirilmeden kalır.

**Geçti:** metin bire bir geri geliyor.

**Ön doğrulama:** `autosave_writes_off_the_ui_thread_and_restores`,
`a_fresh_console_never_overwrites_an_earlier_consoles_file`,
`restored_console_over_20_mb_is_refused_and_kept_on_disk` (console.rs);
`a_fresh_autosave_moves_a_file_it_did_not_read_aside`, `a_late_write_of_a_replaced_autosave_is_dropped`,
`a_restored_text_is_not_written_back` (hacivat-app src/console/autosave.rs). **Otomatik kapsam yok:**
gerçek bir süreç kapanıp yeniden açılması.

## 12. VoiceOver (E14; elle, kapıyı engellemez, L4 M9'da kapılar)

**Yap:** ⌘F5 ile VoiceOver'ı açın. Editöre odaklanıp yukarı/aşağı okla satır satır gezin; Shift+ok ile
bir seçim yapın. Sonra ızgaraya odaklanın (sonuç sekmesine tıklayın ya da Tab).

**İyi görünüm:**
- Editör **SQL editor, pg17** / **SQL düzenleyici, pg17** olarak duyurulur; her satır okunur; seçim
  duyurulur. Bir çalıştırmanın durumu kibarca (polite) duyurulur.
- Izgara bir **grid / tablo** olarak, başlığıyla duyurulur: **Result 1** / **Sonuç 1**. Kaydırma
  durunca görünen bölge bir kez duyurulur.

**Geçti:** satır satır okuma, seçim duyurusu, ızgaranın rolü ve başlığı.

**Ön doğrulama:** `editor_declares_multiline_text_input_with_its_label`,
`children_are_text_runs_with_their_text`, `window_is_plus_minus_100_lines`,
`execution_status_is_announced_politely` (hacivat-ui-editor a11y.rs); `grid_declares_role_and_label`
(`Role::Grid`, "Result 1"), `grid_node_is_the_focused_element`,
`the_visible_region_is_announced_once_it_is_still` (hacivat-ui-grid view.rs). **Otomatik kapsam yok:**
gerçek VoiceOver konuşması.

---

## Demo sonrası ölçümler

1. **E11: bir bağlantı ve açık bir konsolla boşta RSS.** Release derlemeyle açın, `pg17`'ye bağlanıp
   konsolu açık bırakın, 10 saniye dokunmadan bekleyin:

   ```
   ps -o rss= -p "$(pgrep -n -x hacivat)" | awk '{printf "%.0f MiB\n", $1/1024}'
   ```

   Değeri sonuç tablosuna yazın (bağlantısız bütçe 200 MB; bu değer yalnızca kaydedilir).
2. **E2/E3: B1-real ve B3** ekran uyanık, 120 Hz dahili ekranda: komutlar `m3a.md` bölüm 4'tedir.
3. **E7: macOS IME matrisi** `m3a-ime-checklist.md` (UI-EDIT-12 test 7 dahil, satır 1a).

## Temizlik

```
psql "postgresql://hv_test:hv_test_pw@127.0.0.1:54317/hv_test" -c 'DROP TABLE IF EXISTS müşteri'
rm -f ~/hv-demo-m3a.sqlite
```

(`psql` yoksa aynı `DROP TABLE`'ı `pg17` konsolunda çalıştırın; `mysql84`'te de aynısı.) M2b'den kalan
uygulama klasörlerini geri almak isterseniz `.m2b-bak` adlarını geri çevirin (önce M3a demo
klasörlerini silin ya da kenara taşıyın).

## Ekran görüntüleri

Kapı ajanı tam çizimi kendi hedef dizinine yaptı (2.100 görüntü, 0 ihlal; `m3a.md` bölüm 5). Bu
çizim, SDD klasöründeki `baseline-m3a/` ile piksel piksel aynıdır (2.100 görüntünün hepsinde fark
%0,0000); gözden geçirme için `<ana çalışma kopyası>/.superpowers/sdd/2026-11-14-m3-walking-skeleton/baseline-m3a/`
kullanılabilir (kapı ajanının hedef dizini kapıdan sonra silinir). Gözden geçirilecekler: M3a yüzeyleri, 8 `menu-app-*` değişikliği (M2b'ye göre) ve F2'nin üç kararı: iki
satırlı şerit, sarılan (wrap) araç çubuğu ve durum çubuğunun bütün hücreleri sırayla bırakması (give-way
order). Türkçe metinleri bu görüntülerde de gözden geçirin. Onaydan sonra macOS PNG'leri (`baseline-m3a/`)
`design/baselines/m3a/macos/` altına kopyalanır (kapı ajanı bunu yapmadı; Adım 6 PO onayına bağlıdır).

## Sonuç tablosu (PO doldurur)

| Adım | Sonuç (GEÇTİ / KALDI) | Not |
|---|---|---|
| 1 Ana sayfa → bağlı konsol, mono başlık | | |
| 2 Vurgu, şerit, seçici, üzerine gelme, Enter | | |
| 3 İki ifade: Çıktı'da güncelleme sayısı, bir sonuç sekmesi, oluk işaretleri | | |
| 4 1M satır, imleç açık, getirme, kapatma, `SELECT 1` | | |
| 5 ⌘. ≤ 1 s, satırlar kalıyor | | |
| 6 3 × 5 ham TSV, NULL boş | | |
| 7 Üretim: çizgi, renk örneği, uyarı, INSERT reddi | | |
| 8 Açık işlem sorusu, Vazgeç odakta, geri al | | |
| 9 MySQL 8.4, SQLite, `CALL` ilk küme + not | | |
| 10 EN ↔ TR, `1.234,56` | | |
| 11 Yeniden açınca metin geri geliyor | | |
| 12 VoiceOver: editör satır satır, ızgara | | |
| E11 RSS (1 bağlantı + açık konsol): ___ MiB | | |
