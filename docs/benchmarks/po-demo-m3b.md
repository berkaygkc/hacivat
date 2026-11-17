# M3b PO demosu: "Güvenli" kontrol listesi

Bu liste M3b kapısının (Görev 30) yalnızca PO'nun yapabileceği kısmıdır. Adımlar gerçek bir Mac'te,
gerçek Docker sunucularına karşı, gerçek pencere ve gerçek klavyeyle elle çalıştırılır. Her adımda
şunlar vardır: ne yapılacağı (tam SQL ve tuşlar), beklenen ekran (uygulama metinleri `EN` / `TR`),
"geçti"nin ne demek olduğu ve kapı ajanının o adımı önceden hangi otomatik testlerle doğruladığı.

Planın 12 adımı bugünkü davranışa uyarlandı:

- **Üretim yazılabilir, ama yalnızca onayla** (Görev 26). Üretim konsollarındaki salt okunur notu yok.
- **D-043:** üretimde MySQL/MariaDB/TiDB'de veri değiştiren her ifade **gönderilmeden önce** sorulur.
  "Koru / İfadeyi geri al" yalnızca PostgreSQL ve SQLite'tadır (adım 3b).
- Üretimde **"bir daha sorma" yoktur** ve **"Tümünü çalıştır" gösterilmez** (T26 düzeltme turu 2).
- **İşlem rozeti ve kip çipi** (T22): araç çubuğunda `Auto-commit` / `Manual` / `Smart` çipi; açık
  bir kullanıcı işlemi varken `Transaction · n statements` rozeti.

Önceden doğrulama adımın yerini tutmaz. Uygulama testleri sahte (fake) sürücü ve başsız GPUI ile,
Docker testleri arayüzsüz çalışır. Gerçek pencere, gerçek odak, IME ve ekran okuyucu yalnızca bu
listeyle görülür. **Yalnızca PO:** her adımın ekranı. Adım 9 (iptal yükseltme) ve 12 (EN ↔ TR)
özellikle gözle bakılması gereken adımlardır.

**Kapı ajanının ön çalıştırması.** Arayüz başsız sürülemez. Demo SQL'ini doğrudan sunucuda (geri alınan
bir işlem içinde) denemek de bu ortamda izin almadı. Bu yüzden ön doğrulama, her adımın altında adı
geçen uygulama ve Docker testleridir. Bu testler kapıda bu ağaçta yeşil koştu: `docs/benchmarks/m3b.md`
bölüm 2. Bu belgedeki SQL metinleri sunucuda ayrıca denenmedi; ama her ifade biçimi (koşullu `UPDATE`,
filtresiz `DELETE`, `BEGIN` + `UPDATE`, salt okunur `INSERT`, `:id`, `CALL`, hata ilkesi, iptal) aşağıda
adı geçen bir Docker testinde gerçek sunucuda koşar.

Kapı kaydı: `docs/benchmarks/m3b.md` (İngilizce).

## 0. Hazırlık

1. **Docker yığını** (`hacivat-m2a`) ayakta. Kontrol (yalnızca okur):

   ```
   cargo xtask docker status --set full
   ```

   İyi görünüm: `m2a-*` kapsayıcılar `healthy`. Docker testleri koşarken demo yapmayın: aynı
   sunucuları kullanırlar.

2. **Uygulama** (release; adım 12 dışında İngilizce başlar):

   ```
   cargo run --release -p hacivat -- --lang en
   ```

3. **Test sunucuları** (yalnızca demo içindir, gerçek parola değildir; `docker/m2a/README.md`):
   kullanıcı `hv_test`, parola `hv_test_pw`, veritabanı `hv_test`; salt okunur kullanıcı `hv_ro`
   (aynı parola). Portlar (`127.0.0.1`): `pg17` 54317, `mysql84` 33084.

4. **Profiller** (⌘N, "Save and connect"):

   | Profil | Motor | Port | Kullanıcı | Ortam (Environment / Ortam) | Not |
   |---|---|---|---|---|---|
   | `pg17` | PostgreSQL | 54317 | `hv_test` | `dev` | kurulum, adım 6, 8, 10 |
   | `pg17-prod` | PostgreSQL | 54317 | `hv_test` | **`production` / `üretim`** | adım 1–4, 9, 11 |
   | `pg17-ro` | PostgreSQL | 54317 | `hv_ro` | `dev`, **Read-only / Salt okunur** açık | adım 5 |
   | `mysql84` | MySQL | 33084 | `hv_test` | `dev` | adım 7 |
   | `mysql84-prod` | MySQL | 33084 | `hv_test` | **`production`** | adım 3b |

5. **Demo tablosu.** `pg17` (dev) konsolunda çalıştırın (Whole script / Tüm betik, ya da hepsini seçip ⌘↵):

   ```sql
   CREATE TABLE IF NOT EXISTS demo_orders (id int PRIMARY KEY, city text NOT NULL, total numeric(12,2) NOT NULL);
   DELETE FROM demo_orders WHERE id BETWEEN 1 AND 5;
   INSERT INTO demo_orders VALUES (1, 'İstanbul', 1234.56), (2, 'İzmir', 7.00), (3, 'Şanlıurfa', 1234567.50), (4, 'Ankara', 42.00), (5, 'Iğdır', 0.10);
   SELECT * FROM demo_orders ORDER BY id;
   ```

   Aynısını `mysql84` (dev) konsolunda da çalıştırın (`text` → `varchar(64)`); MySQL'de tablo InnoDB olur:

   ```sql
   CREATE TABLE IF NOT EXISTS demo_orders (id int PRIMARY KEY, city varchar(64) NOT NULL, total decimal(12,2) NOT NULL);
   DELETE FROM demo_orders WHERE id BETWEEN 1 AND 5;
   INSERT INTO demo_orders VALUES (1, 'İstanbul', 1234.56), (2, 'İzmir', 7.00), (3, 'Şanlıurfa', 1234567.50), (4, 'Ankara', 42.00), (5, 'Iğdır', 0.10);
   ```

   (Dev bağlantıda `DELETE … WHERE` sorusuz koşar. `WHERE`'siz bir `DELETE` dev'de bile bir kez sorardı:
   onaylı Q1 kuralı.)

6. Hatırlatmalar: Çalıştır ⌘↵; durdur ⌘.; geçmiş ⌘⇧H. Escape hiçbir zaman iptal etmez ve hiçbir
   zaman "çalıştır" demez. Bir dialog nasıl kapanırsa kapansın (Escape, X, ⌘W, pencereyi kapatmak,
   çıkmak) güvenli yanıtı verir: `Cancel` / `Vazgeç`, `Roll back` / `Geri al`. Bir şey farklıysa adım
   numarasıyla not alın ve sondaki tabloya işleyin.

---

## 1. Üretimde `SELECT` sorusuz çalışır

**Yap:** `pg17-prod`'a bağlanın. Konsolda:

```sql
SELECT * FROM demo_orders ORDER BY id;
```

⌘↵.

**İyi görünüm:**
- Hiçbir dialog açılmaz; 5 satır bir sonuç sekmesine gelir.
- Konsolda üretim izi (kırmızı çizgi/ortam etiketi) var; **salt okunur notu yok** (T26 sildi).
- Kip çipi **`Manual`** / **`Manuel`** der. Çipin menüsünde not: `Production consoles start in Manual.` /
  `Üretim konsolları Manuel kipte başlar.` (D-036).

**Geçti:** sorusuz sonuç, salt okunur notu yok, çip `Manual`.

**Ön doğrulama:** Docker `production_select_runs_without_a_dialog` (`hacivat-exec/tests/docker_prod_unlock.rs`);
`a_production_insert_is_confirmed_before_it_is_kept` (`cross_surface_m3a.rs`); görüntüler
`console-production-*`, `tx-chip-production-*`.

## 2. `UPDATE … WHERE id = 3` → "1 row changed — Commit / Roll back", geri sayım; Roll back

Üretim Manual başladığı için önce çipi `Auto-commit`'e alın: plan bu adımda Hacivat'ın **kendi**
işlemini gösterir ve bu biçim yalnızca **boşta, Auto kipte** seçilir (P-M3b-1).

**Yap:** Kip çipine tıklayın → `Auto-commit` / `Otomatik commit`. Sonra:

```sql
UPDATE demo_orders SET total = total + 1 WHERE id = 3;
```

⌘↵. Dialog açılınca geri sayımı birkaç saniye izleyin, sonra `Roll back` / `Geri al`'a basın.

**İyi görünüm:**
- İfade çalışır, **sonra** sorar (veri başka oturumlara görünmez): başlık
  `1 row changed on pg17-prod` / `pg17-prod üzerinde 1 satır değişti`.
- Gövde: `Hacivat ran this statement in a transaction of its own. Other sessions don't see the change until you commit.`
  / `Hacivat bu ifadeyi kendi açtığı bir işlemde çalıştırdı. Commit edene kadar diğer oturumlar değişikliği görmez.`
- `Its rows stay locked while this waits. With no answer, Hacivat rolls back after 5 minutes.` /
  `Yanıt beklenirken satırlar kilitli kalır. Yanıt gelmezse Hacivat 5 dakika sonra geri alır.`
- Geri sayım `Rolls back in 4:59` / `4:59 sonra geri alınır`, her saniye azalır.
- Düğmeler `Commit` / `Commit et` ve `Roll back` / `Geri al`; ilk odak güvenli düğmede.
- Geri aldıktan sonra Output: `Rolled back: the confirmed statement's changes were discarded.` /
  `Geri alındı: onaylanan ifadenin değişiklikleri atıldı.` `SELECT total FROM demo_orders WHERE id = 3`
  hâlâ `1234567.50` verir.

**Geçti:** soru ifadeden sonra, geri sayım işliyor, Roll back sonrası değer değişmemiş. İsteğe bağlı:
Escape ya da ⌘W da `Roll back` demektir (deneyin, değer yine değişmemeli).

**Ön doğrulama:** `pg_production_update_asks_after_it_runs_then_commit_reaches_log_and_gutter` (sürücü
sırası `BEGIN → UPDATE`, Commit'ten sonra log ve oluk), `the_confirm_in_transaction_deadline_undoes_the_statement_and_says_so`
(`cross_surface_m3b.rs`); `commit_prompt_closes_answer_roll_back`, `the_deadline_answers_roll_back`,
`quit_with_an_open_commit_prompt_answers_roll_back` (`confirm.rs`); `auto_idle_confirm_uses_hacivats_own_tx`
(`hacivat-session/tests/confirm_tx.rs`); Docker `production_dml_without_an_answer_leaves_nothing_visible`,
`production_dml_rolls_back_at_the_deadline` (`docker_prod_unlock.rs`), `confirm_in_tx_both_forms_per_engine`
(`docker_m3b_matrix.rs`, 11 sunucu satırı).

## 3. `DELETE FROM demo_orders` → adı yazın; Cancel

**Yap:** (`pg17-prod`, kip hangisi olursa olsun)

```sql
DELETE FROM demo_orders;
```

⌘↵. Ad alanına önce `Demo_orders` yazın (büyük D), sonra silip `demo_orders` yazın. **Sonra
`Cancel` / `Vazgeç`.**

**İyi görünüm:**
- Dialog ifade **gönderilmeden önce** açılır: `Run this statement on pg17-prod?` /
  `Bu ifade pg17-prod üzerinde çalıştırılsın mı?`; etki `Changes data with no usable filter` /
  `Veriyi kullanılabilir bir filtre olmadan değiştirir`; `Nothing is sent until you choose.` /
  `Siz seçene kadar hiçbir şey gönderilmez.`
- Ad alanı: `Type demo_orders to run it` / `Çalıştırmak için demo_orders yazın`,
  `Type it exactly as shown, letter case included.` / `Büyük-küçük harf dahil, tam gösterildiği gibi yazın.`
- `Execute` / `Çalıştır` yalnızca bayt bayt doğru adla etkin olur (`Demo_orders` ile etkin değil).
  Ad alanında Enter hiçbir zaman çalıştırmaz.
- **"Bir daha sorma" düğmesi yok**, `Execute all` / `Tümünü çalıştır` yok (üretim).
- Cancel sonrası hiçbir şey gönderilmez; 5 satır yerinde.

**Geçti:** yanlış harf büyüklüğüyle düğme kapalı; doğru adla açık; Cancel hiçbir şey göndermiyor;
"bir daha sorma" yok.

**Ön doğrulama:** `production_delete_from_t_needs_the_name_typed`, `production_never_offers_dont_ask_again_nor_execute_all`,
`off_production_the_same_dialog_offers_both` (`cross_surface_m3b.rs`); `execute_disabled_until_exact_bytes`,
`the_typed_name_compares_exact_bytes`, `enter_in_type_field_never_executes`, `any_close_answers_cancel`,
`execute_all_is_never_built_on_production` (`confirm.rs`); Docker `docker_m3b_matrix` typed-name satırları
(pg17-tr'de `DELETE FROM müşteri` bayt bayt), `mysql_joined_and_comma_updates_name_the_table_that_changes`.
Görüntüler `confirm-*`.

## 3b. D-043: üretimde MySQL değişikliği gönderilmeden önce sorulur

**Yap:** `mysql84-prod`'a bağlanın. Konsolda:

```sql
UPDATE demo_orders SET total = total + 1 WHERE id = 3;
```

⌘↵ → dialog → `Cancel`. Sonra yeniden ⌘↵ → `Execute`.

**İyi görünüm:**
- Dialog **önce** açılır (PostgreSQL'deki gibi sonra değil): `Run this statement on mysql84-prod?`,
  etki `Changes data` / `Veriyi değiştirir`, `Nothing is sent until you choose.`
- Ardından "Koru / İfadeyi geri al" (Keep/Undo) ya da "Commit / Roll back" sorusu **gelmez**.
- Cancel: hiçbir şey gönderilmez. Execute: ifade çalışır, sonuç Output'ta.
- "Bir daha sorma" yok.

**Geçti:** soru gönderimden önce, sonrasında ikinci soru yok.

**Ön doğrulama:** `mysql_production_update_is_confirmed_before_it_is_sent` (`cross_surface_m3b.rs`);
Docker `production_myisam_dml_is_confirmed_before_it_is_sent`, `myisam_rollback_is_logged_as_not_undone`
(`docker_prod_unlock.rs`, mysql84). İşlemsiz bir tabloda geri alma yapılamazsa Output
`Not undone: the table doesn't support transactions, so the statement's changes stay.` /
`Geri alınmadı: tablo işlemleri desteklemiyor; ifadenin yaptığı değişiklikler kalıyor.` der.

## 4. Manual kip: `BEGIN; UPDATE …` → Keep / Undo; Undo; rozet açık işlemi göstermeye devam eder

**Yap:** `pg17-prod` konsolunda çipi `Manual`'a alın. Şunu seçin ve ⌘↵ (seçim ayrı ifadeler olarak koşar):

```sql
BEGIN;
UPDATE demo_orders SET city = 'İSTANBUL' WHERE id = 1;
```

1. `BEGIN` için bir onay dialogu açılır (üretimde işlem denetimi de sorulur; T28 PO listesi):
   etki `Controls the transaction` / `İşlemi denetler` → `Execute`.
2. `UPDATE` çalışır ve sorar → **`Undo` / `İfadeyi geri al`**.
3. Araç çubuğundaki rozete bakın; sonra rozetten `Roll back` / `Geri al`.

**İyi görünüm:**
- İkinci soru savepoint biçimindedir: `1 row changed inside your transaction` /
  `İşleminizin içinde 1 satır değişti`; satırlar `Keep: the change joins your open transaction. Nothing is committed yet.`
  / `Koru: değişiklik açık işleminize katılır. Henüz hiçbir şey commit edilmez.` ve
  `Undo: removes this statement's change; your transaction stays open.` /
  `İfadeyi geri al: yalnızca bu ifadenin değişikliği kaldırılır; işleminiz açık kalır.`;
  geri sayım `Undoes this statement in 4:59` / `İfade 4:59 sonra geri alınır`.
- Undo sonrası Output: `Undone: the confirmed statement was rolled back, and your transaction is still open.` /
  `Geri alındı: onaylanan ifade geri alındı; işleminiz hâlâ açık.`
- Rozet hâlâ görünür: `Transaction · n statements · <süre>` / `İşlem · n ifade · <süre>`, `Commit` /
  `Commit et` ve `Roll back` / `Geri al` ile. Soru açıkken rozet düğmeleri kapalıdır.
- Rozetten Roll back sonrası rozet kaybolur; `id = 1` hâlâ `İstanbul`.

**Geçti:** BEGIN sorulur; UPDATE'te Keep/Undo; Undo işlemi kapatmaz; rozet kalır; Roll back temizler.

**Ön doğrulama:** `manual_idle_confirm_opens_the_user_tx_then_a_savepoint`,
`user_begin_earlier_in_the_run_forces_the_savepoint_form`, `failed_statement_in_savepoint_form_restores_the_user_tx`
(`confirm_tx.rs`); cross-surface akış 1'in Manual kolu (`SAVEPOINT hv_confirm … RELEASE` sırası);
`badge_only_for_user_transactions`, `badge_answers_wait_while_the_session_runs` (`tx_ui.rs`); Docker
`confirm_in_tx_on_the_matrix` (`docker_m3b_confirm.rs`). Görüntüler `tx-badge-*`, `commit-sp-*`.

## 5. Salt okunur profil: `INSERT` sınıflandırıcının nedeniyle reddedilir

**Yap:** `pg17-ro`'ya bağlanın:

```sql
INSERT INTO demo_orders VALUES (6, 'Bursa', 1);
```

⌘↵.

**İyi görünüm:**
- Dialog yok; hiçbir şey gönderilmez. Output ve oluk işareti:
  `Not sent: this connection is read-only, and the statement changes data.` /
  `Gönderilmedi: bu bağlantı salt okunur ve ifade veriyi değiştiriyor.`
  (Hacivat'ın okuyamadığı bir ifade için: `Not sent: this console is read-only, and Hacivat couldn't confirm that the statement only reads.`
  / `Gönderilmedi: bu konsol salt okunur ve Hacivat ifadenin yalnızca okuduğunu doğrulayamadı.`)
- Sunucu hatası (25006) **görünmez**: reddetme istemcide.
- Kip çipi kapalı; ipucu salt okunur oturumun hep `Auto-commit` olduğunu söyler.

**Geçti:** "Not sent" cümlesi, sunucu hatası yok.

**Ön doğrulama:** `a_production_insert_is_refused_logged_and_marked_failed` (`cross_surface_m3a.rs`,
T18/T24'te yeni davranışa güncellendi); Docker SAFE-06 koşuları A′ (`docker_m3b_bypass.rs`, ~2.000
satır, 0 yanlış red) ve B (`docker_m2b_bypass.rs`), `docker_m3a_readonly` 5/5.

## 6. `SELECT … WHERE id = :id` → değer istemi; Türkçede `1.234` işaretlenir

**Yap:** Uygulamayı Türkçeye alın (başlık çubuğundaki EN/TR). `pg17` konsolunda:

```sql
SELECT * FROM demo_orders WHERE id = :id;
```

⌘↵. Değer alanına `1.234` yazın; uyarıyı okuyun; sonra alanı `3` yapıp `Çalıştır`. Aynı ifadeyi
yeniden çalıştırın.

**İyi görünüm:**
- İstem: `Values for this statement` / `Bu ifadenin değerleri`, `1 parameter` / `1 parametre`; sütunlar
  Ad / Değer / Tür.
- `1.234` için: `Ondalık 1.234 olarak okunur: burada nokta ondalık ayırıcıdır. 1234 demek istediyseniz noktasız yazın.`
  (EN: `Read as the decimal 1.234: a dot is the decimal point here. If you meant 1234, write it without the dot.`);
  `Resolve the flagged value first` / işaretli değer çözülmeden `Çalıştır` çalışmaz.
- Not: `Değerler yalnızca bu uygulama oturumu boyunca tutulur ve geçmişte maskelenir.`
- `3` ile Şanlıurfa satırı gelir. İkinci çalıştırmada değer `remembered` / hatırlanan olarak dolu gelir,
  ama istem **yine açılır** (D8: hatırlanan değer yalnızca doldurur).

**Geçti:** uyarı Türkçe ve doğru; değer bağlanarak gönderilir (metne gömülmez).

**Ön doğrulama:** `turkish_grouped_number_is_flagged_ambiguous` (`exec_ui_m3b.rs`);
`params_prompt_into_a_bound_run` (`cross_surface_m3b.rs`: `$1` Int olarak bağlanır, hatırlanan değer);
`classifier_receives_final_text` (T24: havuz `SELECT … $1` görür); Docker `binds_round_trip_every_type_hint`,
`value_with_quote_is_bound_not_spliced` (`docker_m3b_matrix.rs`).

## 7. MySQL `CALL` üç küme → üç sekme

**Yap:** `mysql84` (dev) konsolunda, **tek başına**:

```sql
CALL hv_test.hv_t14_three();
```

(Docker testlerinin fikstürüdür. Yoksa önce, ayrı bir çalıştırmada:
`CREATE PROCEDURE hv_test.hv_t14_three() BEGIN SELECT 1 AS n; SELECT 2 AS n; SELECT 3 AS n; END`.)

**İyi görünüm:** üç sonuç sekmesi, her birinde tek satır (`n` = 1, 2, 3). 100'den çok küme dönerse
`The statement returned N result sets; the first 100 are shown.` (burada görünmez).

**Geçti:** üç sekme.

**Ön doğrulama:** `a_call_with_three_result_sets_keeps_each_in_its_own_tab` (`exec_ui.rs`, T24'te 3 sekmeye
güncellendi); Docker `multi_result_call_and_run_as_one` (`docker_m3b_matrix.rs`, bütün satırlar);
oturum testleri 150 küme → 100 + `skipped: 50` (`hacivat-session/tests/run_m3b.rs`).

## 8. 5 ifadelik betikte 2. ifade hata verir → çubuk → PostgreSQL'de "Geri al ve devam et"

**Yap:** `pg17` (dev) konsolunda hepsini seçin ve ⌘↵:

```sql
BEGIN;
SELECT 1 / 0;
INSERT INTO demo_orders VALUES (6, 'Bursa', 6.00);
SELECT count(*) FROM demo_orders;
DELETE FROM demo_orders WHERE id = 6;
```

Çubuk görününce **`Roll back and continue` / `Geri al ve devam et`**.

**İyi görünüm:**
- Satır içi çubuk: `Statement 2 of 5 failed, and the transaction failed with it` /
  `2. ifade başarısız oldu (5 ifadeden); işlem de başarısız oldu`.
- Başarısız PG işleminde `Ignore` / `Yok say` ve `Retry` / `Yeniden dene` **yerine** `Roll back and continue`
  vardır; `Stop` / `Durdur` her zaman vardır. F6 çubuğa girer; Tab çubuktan çıkar.
- Geri al ve devam et: `ROLLBACK` gönderilir, kip değişmez, 3–5. ifadeler koşar (sayım 6 gösterir, sonra
  6 silinir).
- Konsolu kapatmak çubuğa `Stop` der.

**Geçti:** çubuk metni doğru; seçenekler PG başarısız işlemine göre; devam ediyor.

**Ön doğrulama:** `error_policy_roll_back_and_continue` (`cross_surface_m3b.rs`; orada 3 ifadeden 2.si);
`error_bar_shows_roll_back_and_continue_in_a_failed_pg_tx`, `error_bar_choice_reaches_the_session`,
`f6_enters_the_bar_and_leaves_it_without_answering`, `closing_the_console_answers_stop` (`escalation.rs`);
`rollback_and_continue_keeps_manual_mode` (oturum); Docker `docker_m3b_matrix` hata ilkesi satırları
(Auto ve Manual'da PG; MySQL ailesi ve SQLite'ta işlem içinde Ignore).

## 9. `SELECT pg_sleep(60)` → Stop, yine Stop → yükseltme dialogu → Beklemeyi sürdür

**Not:** gerçek PostgreSQL `pg_sleep`'i iptal isteğine hemen bırakır, bu yüzden normalde **yükseltme
görünmez**: ⌘. ifadeyi bir saniyeden kısa sürede durdurur. Bu da doğru davranıştır (bölüm A).
Yükseltmeyi görmek için iptalin cevapsız kalması gerekir (bölüm B, isteğe bağlı).

**A. Yap:** `pg17-prod` (ya da `pg17`) konsolunda `SELECT pg_sleep(60);` → ⌘↵ → ⌘.
**İyi görünüm:** ifade iptal edilir, Output'ta iptal satırı; dialog yok. Escape hiçbir şeyi iptal etmez.

**B. (isteğe bağlı; yalnız PO, Docker testleri koşmuyorken)** Sunucuyu kısa süre dondurup iptali
cevapsız bırakın:

1. Konsolda `SELECT pg_sleep(60);` → ⌘↵.
2. Bir terminalde: `docker pause m2a-pg17`
3. ⌘. → 10 saniye bekleyin → toast: `Cancelling is taking long` / `İptal uzun sürüyor`,
   `The server hasn't stopped the statement on pg17-prod after 10 s.` / `10 s geçti; sunucu pg17-prod üzerindeki ifadeyi hâlâ durdurmadı.`,
   `Show options…` / `Seçenekleri göster…`.
4. Yine ⌘. → dialog: `The statement isn't answering the cancel` / `İfade iptale yanıt vermiyor`;
   seçenekler `Keep waiting` / `Beklemeyi sürdür` (odakta), `Terminate on server` / `Sunucuda sonlandır`,
   `Disconnect` / `Bağlantıyı kes`; her birinin sonucunu anlatan satırlar (açık işlemlerin geri alınacağı).
5. **`Keep waiting`** (ya da Escape: o da Beklemeyi sürdür demektir).
6. **Hemen** `docker unpause m2a-pg17`. İptal yerine ulaşır, ifade durur.

**Geçti:** A'da tek ⌘. yeter; B'de toast 10 s'de, ikinci ⌘. dialogu açar, odak Keep waiting'de,
Escape yükseltmez, unpause sonrası her şey normal.

**Ön doğrulama:** `slow_cancel_into_the_dialog_then_keep_waiting` (`cross_surface_m3b.rs`, gerçek 10 s);
`slow_cancel_toast_after_ten_seconds`, `second_press_opens_the_dialog_with_keep_waiting_focused`,
`escape_never_escalates`, `failed_terminate_shows_the_server_text_verbatim`,
`disconnect_from_the_default_console_names_the_others_and_closes_the_profile` (`escalation.rs`); Docker
`cancel_within_one_second_on_every_engine` (M3a). Görüntüler `esc-dialog-*`, `toolbar-cancelling-*`.

## 10. Geçmiş dialogu → `istanbul` ara → ekle ve çalıştır

**Yap:** `pg17` konsolunda önce bir kez çalıştırın:

```sql
SELECT * FROM demo_orders WHERE city = 'İstanbul';
```

Sonra ⌘⇧H. Arama alanına `istanbul` (küçük harf, noktasız `i`) yazın. Satırı seçip
`Insert and run` / `Ekle ve çalıştır`.

**İyi görünüm:**
- Dialog `Query history` / `Sorgu geçmişi`; arama ipucu `Search ignores letter case and accents: musteri finds müşteri.` /
  `Arama büyük-küçük harfi ve aksanları yok sayar: musteri, müşteri'yi bulur.`
- `istanbul` → `İstanbul` içeren satırı bulur (Türkçe katlama). Arama arayüzü dondurmaz.
- Hedef satırı: `Goes into the console pg17; Run checks it there.` / `pg17 konsoluna eklenir; Çalıştır denetimi orada yapılır.`
- Ekle ve çalıştır, metni konsola ekler ve **olağan Çalıştır yolundan** geçirir (bir yazma ifadesi olsaydı
  onay sorulurdu).
- Parola içeren bir ifade geçmişte `redacted` / maskeli görünür ve `Insert and run` kapalıdır.

**Geçti:** Türkçe arama buluyor; çalıştırma kapıdan geçiyor.

**Ön doğrulama:** `history_insert_and_run_meets_the_gate` (`cross_surface_m3b.rs`: geçmişten
`DELETE FROM orders;` → onay → Escape iptal); `history_ui.rs` (12 test); `turkish_search_matches_both_ways`,
`turkish_search_more_pairs`, `unmasked_sql_never_reaches_history` (`hacivat-exec/tests/history.rs`).
Görüntüler `history-*`.

## 11. Boşta açık işlem uyarısı (toast)

Demo derleme bayrağı **yoktur** (T22 sapma 5, T29 taşınan iş). Eşik profilin boşta kalma süresidir
(`idle_disconnect_s`; varsayılan üretimde 1 saat). Demo için kısaltın:

**Yap:**
1. Uygulamayı kapatın. `connections.toml`'u bulun ve `pg17-prod` bölümüne `idle_disconnect_s = 60` ekleyin:

   ```
   find "$HOME/Library/Application Support" -maxdepth 3 -name connections.toml -path '*hacivat*'
   ```

2. Uygulamayı açın, `pg17-prod`'a bağlanın (kip `Manual`). Çalıştırın:

   ```sql
   UPDATE demo_orders SET total = total WHERE id = 2;
   ```

   → `Keep` / `Koru`. Bir dakikadan biraz fazla hiçbir şey yapmayın.
3. Toast'ta **`Keep open` / `Açık bırak`**; sonra rozetten `Roll back`.
4. Bitince `idle_disconnect_s` satırını silin.

**İyi görünüm:**
- Toast: `Transaction open for 1 min on pg17-prod` / `pg17-prod üzerinde 1 min süredir açık bir işlem var`,
  `It may hold locks that block other sessions.` / `Diğer oturumları engelleyen kilitler tutuyor olabilir.`;
  düğmeler `Commit` / `Commit et`, `Roll back` / `Geri al`, `Keep open` / `Açık bırak`.
- Açık işlem varken boşta bağlantı kesilmez (R-M3-73); Keep open işlemi geri almaz.
- Toast bir tıkla kapanmaz ve hiçbir yanıtı kendiliğinden vermez.

**Geçti:** toast eşikten sonra geliyor, Keep open işlemi açık bırakıyor.

**Ön doğrulama:** `idle_toast_keep_leaves_the_transaction_open` (`cross_surface_m3b.rs`, toast doğrudan
gösterilerek); `idle_toast_keep_never_rolls_back`, `the_idle_toast_goes_with_its_transaction`,
`newer_idle_toast_replaces_the_older` (`tx_ui.rs`); `idle_transaction_fires_per_session`
(`hacivat-session/tests/liveness_m3b.rs`). **Otomatik kapsam yok:** gerçek eşikten toast'a uçtan uca yol
(oturum kancası M3c'ye taşındı) ve toast düğmelerinin klavyeyle kullanımı. Gözle ve klavyeyle bakın.

## 12. Her yeni yüzeyde EN ↔ TR

**Yap:** Başlık çubuğundaki EN/TR ile Türkçeye geçin ve 2, 3, 4, 6, 8, 9, 10, 11'deki yüzeylere bakın.
Pencereyi 960 px genişliğe daraltıp üretim konsolunu ve onay dialogunu bir kez daha açın.

**İyi görünüm:**
- Her metin Türkçe; kesik, taşan, üst üste binen etiket yok; üretim adı hiçbir zaman kesilmez.
- Ad yazma alanı `İ`/`i` ayrımını korur (`DELETE FROM müşteri` → `müşteri` bayt bayt).
- Bilinen PO maddeleri (m3b.md bölüm 7): 960 px tr-XT'de üretim bekleme cümlesi 3 satır;
  Türkçe metinlerin hepsi `# Review: PO` işaretli.

**Geçti:** Türkçe metinler doğal ve tutarlı. Önerilerinizi not edin.

**Ön doğrulama:** 3.244 görüntü, 0 ihlal (en, en-XA, tr, tr-XT × koyu/açık × 1280/960); `cargo xtask l10n check`
0 hata; `badge_and_segment_are_never_clipped_at_960_tr_xt` (`tx_ui.rs`); güven kontrolü kesik üretim
adında başarısız olur (T29 I2).

---

## Sonuçlar

| Adım | Geçti / Kaldı | Not |
|---|---|---|
| 1 Üretimde SELECT | | |
| 2 Commit / Roll back, geri sayım | | |
| 3 DELETE, ad yazma, Cancel | | |
| 3b D-043 MySQL önce sorar | | |
| 4 Manual Keep / Undo, rozet | | |
| 5 Salt okunur INSERT reddi | | |
| 6 `:id` istemi, `1.234` | | |
| 7 CALL üç sekme | | |
| 8 Geri al ve devam et | | |
| 9 Stop, yükseltme | | |
| 10 Geçmiş, `istanbul` | | |
| 11 Boşta işlem toast'ı | | |
| 12 EN ↔ TR | | |

Bittiğinde: `demo_orders` tablolarını isterseniz `DROP TABLE demo_orders;` ile silin (dev profilinden).
