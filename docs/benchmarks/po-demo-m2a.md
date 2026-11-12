# M2a PO demosu: bağlantı akışı kontrol listesi

Bu liste M2a kapısının (Görev 22) yalnızca PO'nun yapabileceği kısmıdır. 15 adımın tamamı, gerçek bir
Mac'te, gerçek Docker sunucularına ve gerçek Keychain'e karşı elle çalıştırılır. Her adımda üç şey
vardır: ne yapılacağı (tam komutlarla), "iyi" görünümün neye benzediği ve bu adımın hangi otomatik
testle önceden doğrulandığı.

Önceden doğrulama, adımın yerini tutmaz. Testler sahte (fake) sürücü, bellek içi gizli depo ve
başsız (headless) GPUI ile çalışır. Gerçek Keychain, gerçek pencere sistemi ve gerçek sunucu
davranışı yalnızca bu listeyle görülür.

## 0. Hazırlık

1. Docker yığını (`hacivat-m2a`) kapı ajanı tarafından ayakta bırakıldı. Kontrol için (yalnızca
   okur, hiçbir şeyi başlatmaz ya da durdurmaz):

   ```
   cargo xtask docker status --set full
   ```

   İyi görünüm: 13 servisin hepsi `healthy`. Yığın kapalıysa, depo kökünden şunu çalıştırın:

   ```
   cargo xtask docker up --set full
   ```

2. Temiz bir ilk açılış için, varsa eski uygulama klasörlerini kenara taşıyın. Adım 1 ilk açılışı
   görmek ister (boş gezgin). Klasörler `directories` kütüphanesinin macOS konumlarıdır:

   ```
   mv "$HOME/Library/Application Support/dev.hacivat.Hacivat" "$HOME/Library/Application Support/dev.hacivat.Hacivat.bak"
   mv "$HOME/Library/Caches/dev.hacivat.Hacivat" "$HOME/Library/Caches/dev.hacivat.Hacivat.bak"
   ```

   Klasör yoksa komut hata verir; bu sorun değildir, devam edin.

3. Uygulama şu komutla açılır (adım 11 hariç hep İngilizce arayüzle; metinler bu liste ile
   birebir eşleşsin diye):

   ```
   cargo run -p hacivat -- --lang en
   ```

   `.cargo/config.toml` içindeki macOS koşturucusu (`scripts/dev-run.sh`) ikiliyi "Hacivat Dev"
   kimliğiyle imzalar. Bu yüzden Keychain, yeniden derlemeden sonra bile tekrar tekrar izin
   sormaz. Geliştirme derlemesi Keychain'e şu adla yazar:

   - Servis (Keychain Access'te "Ad / Name" sütunu): `hacivat-dev/<profil-uuid>/db-password`
   - Hesap (Account): `hacivat`
   - Yorum (Comment): profilin adı

   Keychain Access'te bakmak için: Keychain Access uygulamasını açın, sol üstte `login`
   anahtar zincirini ve "Passwords / Parolalar" kategorisini seçin, arama kutusuna `hacivat-dev`
   yazın.

4. Test sunucularının ortak bilgileri (yalnızca bu demo içindir, gerçek parola değildir):
   kullanıcı `hv_test`, parola `hv_test_pw`, veritabanı `hv_test` (ayrıca `orders` ve
   `orders_archive`). Portlar: `pg13` 54313, `pg17` 54317, `mysql84` 33084.

5. Aşağıda "iyi görünüm" olarak yazılan her şey birlikte aranmalıdır. Bir şey farklıysa adım
   numarasıyla birlikte not alın. Sonuçları bu dosyanın sonundaki tabloya işleyin.

---

## 1. İlk açılış ve boş durum

**Yap:** Uygulamayı açın. Gezgin panelini inceleyin. Sonra ⌘N'ye basın.

**İyi görünüm:**
- Gezgin "No connections yet" boş durumunu ve üç giriş noktasını (yeni bağlantı, URL yapıştır,
  `pg_service.conf` içe aktar) gösterir.
- ⌘N, 640 px genişliğinde bir iletişim kutusu açar; odak **Host** alanındadır (imleç orada yanıp
  söner).

**Ön doğrulama:** `empty_state_offers_three_entry_points` (connections_explorer),
`new_dialog_is_640_and_focuses_host` (connection_dialog).

## 2. URL yapıştırma

**Yap:** Şu satırı panoya kopyalayıp **Host** alanına yapıştırın (⌘V):

```
postgres://hv_test:hv_test_pw@localhost:54317/orders?sslmode=prefer&application_name=demo&foo=bar
```

**İyi görünüm:**
- Alanlar dolar ve dolan alanlar renk tonuyla işaretlenir (tint).
- Bildirim tam olarak **beş satır** listeler (P37): doldurulan alanlar, parola, `sslmode`,
  `application_name`, `foo`.
- Geri Al (Undo) yapıştırmayı geri alır. Sonra tekrar yapıştırın, aynı sonuç gelir.
- Parola alanı noktalarla maskelidir; URL satırında parola `****` görünür, gerçek parola hiçbir
  yerde düz metin olarak görünmez.

**Ön doğrulama:** `paste_url_into_host_fills_tints_and_explains`, `paste_into_password_field_is_literal`,
`url_row_mirrors_and_masks`, `a_pasted_password_never_reaches_the_url_row_or_the_clipboard`
(connection_dialog).

## 3. Kaydet ve bağlan (yerel)

**Yap:** "Save and connect" düğmesine ya da ⌘↵'ye basın. Bağlanırken ve bağlandıktan sonra başlık
çubuğundaki kapsülü ve alt durum çubuğunu izleyin. Sonra "unencrypted" yazan parçaya tıklayın.

**İyi görünüm:**
- `localhost` olduğu için **ortam sorusu çıkmaz**; ortam kendiliğinden `local` olur.
- Kapsül önce "Connecting" (adım adı ve saniye sayacı, örneğin `0.12 s`), sonra "Connected" ve
  `local` gösterir; yanında "unencrypted" yazar (yerel, TLS yok).
- Durum çubuğu kapsülü birebir yansıtır.
- "unencrypted" parçasına tıklamak açıklama balonunu (popover) açar.
- Gezgindeki satır bağlı işaretini ve gecikmeyi gösterir.

**Ön doğrulama:** `save_local_profile_without_prompt`, `save_and_connect_saves_then_connects`
(connection_dialog); `capsule_and_status_follow_the_current_connection`,
`unencrypted_segment_opens_explanation`, `the_explanation_has_a_keyboard_path` (connections_live).

## 4. Uzak ana bilgisayar ve üretim istemi

**Önce `/etc/hosts` satırını ekleyin** (yönetici parolası ister):

```
sudo sh -c 'echo "127.0.0.1 db-prod-us.acme.io" >> /etc/hosts'
dscacheutil -flushcache
```

Doğrulamak için: `grep acme /etc/hosts` çıktısında yalnızca bu satır görünmeli.

**Yap:** ⌘N. Ad: boş bırakın. Host: `db-prod-us.acme.io`, Port: `54317`, Kullanıcı `hv_test`,
parola `hv_test_pw`, Veritabanı `orders`. Kaydedin.

**İyi görünüm:**
- Ortam istemi açılır ve **Production** önceden seçilidir; altında
  "Suggested because the host name contains “prod”." yazar.
- "Save as production" ile kaydedin. Bağlanın: kapsül dolu (filled) **PRODUCTION** rozetini
  gösterir ve başlık çubuğunda **tek bir** dolu rozet vardır.

**Satırı hemen silmeyin:** adım 14 de aynı ana bilgisayar adını kullanır. Satırı adım 14'ün
sonunda (ya da en geç demo bitince) silin; silme komutu adım 14'ün "Temizlik" bölümündedir.

**Ön doğrulama:** `production_prompt_preselects_and_names_the_result`,
`production_prompt_names_a_name_match` (connections_errors);
`save_remote_asks_environment_then_capsule_turns_production` (cross_surface_m2a);
`remote_save_waits_for_an_environment`, `environment_hint_rules` (connection_dialog).
Tek rozet kuralı kapsülle sınırlıdır: `assert_one_filled_badge` (cross_surface_m2a).

## 5. Bağlantıyı sına (⌘T)

**Yap:** Yeni bir taslakta (kaydetmeden; örneğin adım 2'deki URL'yi yapıştırıp) ⌘T'ye basın.

**İyi görünüm:**
- İzleme (trail) satırları adım adı ve süreyle akar; çalışırken saniye sayacı görünür.
- Sonuç "Connected in N ms" (N tam milisaniye), altında "Nothing was saved." notu.
- Tanılama (diagnostics) satırları görünür.
- "Copy report" panoya **İngilizce** metin koyar. (Bunu TR arayüzde de denemek için adım 13'ten
  sonra tekrar edebilirsiniz.) Yapıştırıp kontrol edin: içinde parola **yok**.
- Sınama bir şey saklamaz: Keychain'de yeni giriş **oluşmaz**.

**Ön doğrulama:** `ok_strip_shows_total_and_diagnostics`, `running_strip_ticks_in_region_format`,
`copy_report_is_english_in_a_turkish_ui`, `stop_cancels_the_running_test` (connections_errors).

## 6. Yanlış parola

**Yap:** Yeni bağlantı: Host `localhost`, Port `54317`, Kullanıcı `hv_test`, Veritabanı `hv_test`,
**yanlış** bir parola (örneğin `yanlis`). "Save and connect".

**İyi görünüm:**
- Hata iletişim kutusu açılır; izlemede ✕ işareti **Sign in** adımındadır.
- "Enter password again" ile parolayı yeniden girin. Sayaç her reddedilişte artar: 1, 2, 3. Üçüncü
  reddedişte durur (tek bağlantı hareketi için en çok 3 sunucu reddi).
- Doğru parolayı (`hv_test_pw`) girince bağlanır ve iletişim kutusu kapanır.
- Girilen parola ekranda ve panoda hiçbir yerde görünmez.

**Ön doğrulama:** `auth_invalid_reenter_counts_and_stops_at_three`,
`a_reentered_password_that_works_closes_the_dialog`,
`a_reentered_password_never_reaches_the_screen_or_the_clipboard`,
`an_empty_reentry_sends_nothing` (connections_errors).

## 7. Olmayan veritabanı: "Use orders"

**Yap:** Adım 3'teki profili düzenleyin (⌘E), Veritabanı alanını `orders_v2` yapın, "Save and
connect".

**İyi görünüm:**
- Hata: veritabanı bulunamadı. Çözüm düğmeleri arasında en çok üç "Use …" önerisi vardır
  ("Use orders", "Use orders_archive", "Use hv_test").
- "Use orders" Veritabanı alanını doldurur ve bağlanır.

**Ön doğrulama:** `unknown_database_offers_top_three`, `an_empty_suggestion_list_offers_no_use_buttons`
(connections_errors); `inline_test_error_fix_use_database` (cross_surface_m2a).

## 8. Desteklenmeyen sürüm: `pg13`

**Yap:** Yeni bağlantı: Host `localhost`, Port `54313`, Kullanıcı `hv_test`, parola `hv_test_pw`,
Veritabanı `hv_test`. "Save and connect".

**İyi görünüm:**
- Hata: sunucu sürümü desteklenmiyor (`server.unsupported_version`); izlemede ✕ **Check server**
  adımındadır. Düğmeler: "Supported versions", "Edit connection", "Connect anyway, unsupported".
- "Connect anyway, unsupported" bağlanır. Kapsülde (ve durum çubuğunda) **unsupported** işareti
  kalıcı olarak görünür. Uygulamayı kapatıp açınca da profilde kalır.
- "Supported versions" düğmesi bir yer tutucu adres açar (`https://hacivat.dev/docs/engines`);
  bu adres henüz yayında olmayabilir (açık madde, bkz. `m2a.md`).

**Ön doğrulama:** `unsupported_offers_ghost_connect_anyway_and_persists` (connections_errors).

## 9. MySQL 8.4, TLS ve silme

**Yap:** Yeni bağlantı: Motor MySQL, Host `localhost`, Port `33084`, Kullanıcı `hv_test`, parola
`hv_test_pw`, Veritabanı `hv_test`. "Save and connect".

**İyi görünüm (bağlanma):**
- Sunucu kendinden imzalı sertifika sunduğu için `Auto` modda hata: `tls.untrusted`.
- "Encrypt without verifying" düğmesine basın (altında sunucunun kimliğinin doğrulanmadığını söyleyen
  not vardır). Bağlanır; kapsülde **TLS 1.3 · not verified**.

**İyi görünüm (silme):**
- Önce Keychain Access'te `hacivat-dev` aratın: bu profilin `hacivat-dev/<uuid>/db-password` girişi
  vardır (hesap `hacivat`, yorum profil adı). `<uuid>` değerini not edin.
- Gezgindeki profilde ⌫ (ya da sağ tık, "Delete…"). Onay kutusu **alertdialog**'dur: başlık
  "Delete … ?", **Cancel düğmesi odaktadır**, "Delete" menüde ve kutuda kırmızıdır.
- "Delete" deyince satır kaybolur. Keychain Access'te aramayı yenileyin (⌘R ya da yeniden arayın):
  `hacivat-dev/<uuid>/db-password` girişi **artık yoktur**.
  Komut satırından kontrol: `security find-generic-password -s "hacivat-dev/<uuid>/db-password"`
  çıktısı "could not be found" demeli.

**Ön doğrulama:** `tls_untrusted_encrypt_without_verifying` (connections_errors);
`delete_confirms_with_cancel_focused_and_removes_secrets`, `only_delete_is_red_in_the_connection_menu`
(connections_explorer); `a_keyring_entry_left_behind_by_a_delete_is_named` (cross_surface_m2a);
`delete_while_connecting_cancels_and_forgets_secrets` (connections_live). Gerçek Keychain silme
turu (E4): `live_keyring_round_trip`, bkz. `m2a.md`.

## 10. Hatırlama seçenekleri

**Yap:** Adım 3'teki `orders` profilini düzenleyin (⌘E). "Remember" seçimi varsayılan **Forever**
(Keychain) olmalıdır. Bunu **For this session** yapıp Kaydedin.

**İyi görünüm:**
- Kaydettiğinizde Keychain'deki `hacivat-dev/<uuid>/db-password` girişi **silinir** (Keychain
  Access'te yenileyin).
- Bağlantıyı kesin (gezgin satırında ↵ ile aç/kapa), sonra yeniden bağlanın: parola istemi çıkar
  ("Sign in to …", "It's kept only if the connection works.").
- Doğru parolayla bağlanınca bu oturum için bellekte tutulur; Keychain'e yazılmaz.

**Ön doğrulama:** `edit_mode_titles_and_retention_change`, `remember_defaults_to_forever_and_disables_without_keyring`
(connection_dialog); `interaction_round_trips_from_the_core_runtime` (connections_errors).
Parolanın diskte durmadığı: `connections_toml_never_contains_a_secret` (E9).

## 11. Gezgin klavyesi ve Türkçe süzgeç

**Yap (İngilizce):** Gezgin listesine odaklanın (⌘1 ile paneli açın, ⌘F ile süzgeç, Tab ile
listeye geçin). Sırayla: ↑/↓ seçim, ↵ bağlan/kes, ⌘E düzenle, ⌘D çoğalt, ⌘⇧C URL'yi kopyala,
⌫ sil (onay çıkar, Cancel ile vazgeçin), Shift-F10 bağlam menüsü.

**Yap (Türkçe süzgeç):** Uygulamayı `cargo run -p hacivat -- --lang tr` ile yeniden açın. `Müşteri-prod`
adlı bir profil oluşturun (örneğin Host `localhost`, Port `54317`). Süzgeç kutusuna `musteri`
yazın.

**İyi görünüm:**
- Her kısayol belirtilen işi yapar; ⌘⇧C panoya **parolasız** URL koyar ve bir bildirim
  "The password isn't included." der (Türkçe arayüzde karşılığı).
- Türkçe süzgeç `musteri` ile `Müşteri-prod` satırını bulur (büyük/küçük harf ve ASCII katlama
  toleranslı).
- Süzgeçte yazarken ⌫ profili **silmez**; yalnızca metni siler.

**Ön doğrulama:** `keyboard_paths`, `context_menu_items`, `filter_is_turkish_tolerant_for_tr`,
`backspace_in_filter_does_not_delete`, `explorer_commands_never_act_on_a_hidden_row`
(connections_explorer); `copy_url_never_includes_password` (connection_dialog).

## 12. Ulaşılamayan ana bilgisayarda Esc

**Yap:** Yeni bağlantı: Host `10.255.255.1`, Port `5432`, Kullanıcı `hv_test`, parola
`hv_test_pw`, Veritabanı `hv_test`. "Save and connect". Kapsül "Connecting" iken **Esc**'e basın.
(İstenirse ortam sorusuna `dev` deyin.)

**İyi görünüm:**
- Bağlanma hemen iptal olur; "Connecting to … was cancelled." başlıklı bir bildirim (toast) çıkar.
- Takılı kalan bir "Connecting" durumu ya da arkada açık kalan bir hata iletişim kutusu yoktur;
  Esc iptali, hata saymaz.

**Ön doğrulama:** `escape_cancels_connect_and_toasts` (connections_live);
`escape_in_the_filter_runs_the_escape_hook` (connections_explorer).

## 13. EN ↔ TR geçişi, iletişim kutusu açıkken

**Yap:** ⌘N ile iletişim kutusunu açın ve birkaç alanı doldurun (ad, host `db.acme.io`, port). Kutu açıkken
dili Türkçeye çevirmeyi deneyin: macOS menü çubuğunda *Hacivat → Settings…* satırına ya da ⌘,'ye
basın, Language bölümünden Türkçe'yi seçin. Sonra tersini yapın (Türkçeden İngilizceye).

Kutu modal bir katmandır ve modal açıkken yalnızca o katmanın kısayolları ile ⌘Q çalışır (P47). Bu
yüzden ⌘, kutu açıkken **çalışmayabilir**. Böyle olursa bunu not edin ("dil, modal açıkken
arayüzden değişmiyor"), kutuyu kapatıp dili ayarlardan değiştirin, kutuyu yeniden açıp etiketleri
kontrol edin. Testin kapsadığı davranış (dil olayı kutu açıkken gelir, değerler kalır) arayüz
yolundan bağımsızdır.

**İyi görünüm:**
- Dil değişimi sırasında yazdığınız değerler **kaybolmaz** (testte kutu açıkken dil değişir).
- Her etiket, ipucu ve düğme Türkçeleşir; **İngilizce kalıntı yoktur** ("Save", "Test connection",
  "Remember", "Environment" gibi). İngilizce kalıntı görürseniz ekran görüntüsü alıp not edin.
- Türkçe metinlerin yazımı ve anlamı PO inceleme listesindedir (`m2a.md`, bölüm 7.2, "PO copy review").

**Ön doğrulama:** `language_switch_keeps_typed_values` (connection_dialog),
`tr_ui_end_to_end_strings`, `the_leak_check_finds_english_text` (cross_surface_m2a). Sözcük
sızıntısı denetimi `english_leaks` ile kendi denetimini de sınar.

## 14. `pg_service.conf` içe aktarma

**Hazırlık:** Örnek dosya (yalnızca bu demo için, dosya izni 600):

```
cat > "$HOME/hv-demo-pg_service.conf" <<'EOF'
[demo_local]
host=localhost
port=54317
dbname=orders
user=hv_test
password=hv_test_pw

[demo_remote]
host=db-prod-us.acme.io
port=54317
dbname=orders
user=hv_test
EOF
chmod 600 "$HOME/hv-demo-pg_service.conf"
```

Uygulamayı bu dosyayla açın (uzak satır yalnızca ortam kuralını sınar; ona bağlanmanız
gerekmez):

```
PGSERVICEFILE="$HOME/hv-demo-pg_service.conf" cargo run -p hacivat -- --lang en
```

**Yap:** ⌘K ile komut paletini açın, "Import pg_service.conf" yazıp seçin.

**İyi görünüm:**
- 480 px'lik iletişim kutusu iki servisi onay kutularıyla listeler.
- **demo_remote** satırı için ortam seçilene dek "Import" düğmesi pasiftir; demo_remote için
  **Production** önceden seçili gelir (ana bilgisayar adı `prod` içeriyor). demo_local için
  ortam gerekmez (yerel).
- Ortamı onayladıktan sonra "Import" ile içe aktarın; iki profil gezginde görünür.
- `demo_local` parolası Keychain'e girer (`hacivat-dev/<uuid>/db-password`). Dosyanın izni 644 ise
  ("chmod 644") parola **yoksayılır** ve "The password of demo_local was ignored because other
  users can read the file." uyarısı çıkar. Bunu da bir kez deneyin.

**Ön doğrulama:** `pg_service_import_requires_env_for_remote`, `an_imported_name_that_is_taken_gets_a_suffix`
(connections_explorer).

**Temizlik:**

```
rm "$HOME/hv-demo-pg_service.conf"
sudo sed -i '' '/db-prod-us.acme.io/d' /etc/hosts
dscacheutil -flushcache
grep -c acme /etc/hosts   # 0 yazmalı
```

## 15. Bağlıyken çıkış

**Yap:** En az bir bağlantı açıkken ⌘Q.

**İyi görünüm:**
- Uygulama **2 saniye içinde** kapanır (takılma, "yanıt vermiyor" yok).
- Terminalde `cargo run` hemen döner. `docker ps` hâlâ 13 `m2a-*` kapsayıcı gösterir; sunucu
  tarafında asılı oturum kalmaz: `docker exec m2a-pg17 psql -U hv_test -d hv_test -c "select count(*) from pg_stat_activity where application_name = 'demo'"`
  sonucu 0 olmalı (adım 2'nin `application_name=demo` bağlantısı kapanmış olmalı).

**Ön doğrulama:** `quit_closes_every_connection`, `quit_during_connect_cancels_within_budget`,
`quit_drains_queued_profile_writes` (connections_live).

---

## Demo sonrası zorunlu kontroller

1. `/etc/hosts` temiz mi: `grep -c acme /etc/hosts` çıktısı `0`.
2. **E9, diskte gizli yok:** hiçbir yerde `hv_test_pw` kalmamalı:

   ```
   grep -r hv_test_pw "$HOME/Library/Application Support/dev.hacivat.Hacivat" "$HOME/Library/Caches/dev.hacivat.Hacivat"
   ```

   Beklenen: çıktı **boş** (çıkış kodu 1). (`$HOME/hv-demo-pg_service.conf` bilerek kapsam dışıdır
   ve adım 14'te silindi.)
3. Keychain Access'te `hacivat-dev` aratın: silinen profillerin girişi kalmamalı; yalnızca
   hâlâ var olan "Forever" profillerinin girişi olmalı.
4. Kalan demo girişlerini silmek için Keychain Access'te `hacivat-dev` girişlerini silebilirsiniz.

## Sonuç tablosu (PO doldurur)

| Adım | Sonuç (GEÇTİ / KALDI) | Not |
|---|---|---|
| 1 Boş durum, ⌘N | | |
| 2 URL yapıştırma | | |
| 3 Kaydet ve bağlan | | |
| 4 Uzak host, PRODUCTION | | |
| 5 Sına, rapor | | |
| 6 Yanlış parola | | |
| 7 Use orders | | |
| 8 pg13 unsupported | | |
| 9 MySQL TLS, silme, Keychain | | |
| 10 Hatırlama | | |
| 11 Klavye, Türkçe süzgeç | | |
| 12 Esc ile iptal | | |
| 13 EN ↔ TR | | |
| 14 pg_service içe aktar | | |
| 15 Çıkış ≤ 2 s | | |
| E9 grep boş | | |
