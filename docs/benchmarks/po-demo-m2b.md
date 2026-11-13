# M2b PO demosu: güvenli ağ ve oturumlar kontrol listesi

Bu liste M2b kapısının (Görev 25) yalnızca PO'nun yapabileceği kısmıdır. 13 adım gerçek bir Mac'te,
gerçek Docker sunucularına, gerçek SSH bastion'larına ve gerçek Keychain'e karşı elle çalıştırılır.
Her adımda dört şey vardır: ne yapılacağı (tam tıklama, alan değeri ve komutlarla), nereye
bakılacağı, beklenen metin (EN ve TR) ve "geçti"nin ne demek olduğu. Her adımın sonunda kapı
ajanının o adımı başsız (headless) olarak nasıl önceden doğruladığı yazılıdır.

Önceden doğrulama adımın yerini tutmaz. Uygulama testleri sahte (fake) sürücü, bellek içi gizli
depo ve başsız GPUI ile çalışır; Docker testleri arayüzsüz çalışır. Gerçek pencere, gerçek
iletişim kutuları ve gerçek Keychain yalnızca bu listeyle görülür.

Kapı kaydı: `docs/benchmarks/m2b.md` (İngilizce). Bu listedeki önceden doğrulama sonuçları oradaki
bölüm 5'te de vardır.

## 0. Hazırlık

1. **Docker yığını** (`hacivat-m2a`) ayakta bırakıldı. Kontrol (yalnızca okur):

   ```
   cargo xtask docker status --set full
   ```

   İyi görünüm: 17 hedef ve 2 yardımcı servis (`ssh-client-keys`, `mysql-x509-certs`), toplam 19
   `m2a-*` kapsayıcı `healthy`. Yığın kapalıysa depo kökünden `cargo xtask docker up --set full`.

2. **Temiz ilk açılış.** Eski uygulama klasörlerini kenara taşıyın (M2a demosundaki gibi; klasör
   yoksa komut hata verir, sorun değil):

   ```
   mv "$HOME/Library/Application Support/dev.hacivat.Hacivat" "$HOME/Library/Application Support/dev.hacivat.Hacivat.m2a-bak"
   mv "$HOME/Library/Caches/dev.hacivat.Hacivat" "$HOME/Library/Caches/dev.hacivat.Hacivat.m2a-bak"
   ```

   Hacivat'ın kendi bilinen sunucular dosyası (`known_hosts`) da bu klasörün içindedir; böylece
   SSH adımları "ilk bağlantı" ile başlar. `~/.ssh/known_hosts` dosyanıza Hacivat **hiç yazmaz**.

3. **Demo SSH anahtarları ve `~/.ssh/config` örneği.** Docker'ın ürettiği istemci anahtarları
   `target/docker` altında 0644 izinlidir. Hacivat başkalarının okuyabildiği bir özel anahtarı
   (OpenSSH gibi) reddeder, bu yüzden önce 0600 kopyalarını alın. Depo kökünden:

   ```
   mkdir -p ~/.ssh && chmod 700 ~/.ssh
   cp target/docker/ssh-client-ed25519     ~/.ssh/hv-demo-ed25519     && chmod 600 ~/.ssh/hv-demo-ed25519
   cp target/docker/ssh-client-ed25519-enc ~/.ssh/hv-demo-ed25519-enc && chmod 600 ~/.ssh/hv-demo-ed25519-enc
   [ -f ~/.ssh/config ] && cp ~/.ssh/config ~/.ssh/config.hv-demo.bak
   { printf '%s\n' '# >>> hacivat-demo' 'Host hv-sshd-a' '  HostName 127.0.0.1' '  Port 2222' \
       '  User hv_ssh' '  IdentityFile ~/.ssh/hv-demo-ed25519' '  IdentitiesOnly yes' '# <<< hacivat-demo' ''; \
     cat ~/.ssh/config 2>/dev/null; } > ~/.ssh/config.hv-demo.new && mv ~/.ssh/config.hv-demo.new ~/.ssh/config
   chmod 600 ~/.ssh/config
   ```

   Blok dosyanın **başına** eklenir: `~/.ssh/config` ilk değeri kullanır, sonda kalsaydı sizin
   `Host *` satırlarınız onu ezebilirdi. Blok yalnızca `hv-sshd-a` takma adına uygulanır. Şifreli
   anahtarın parola ifadesi (passphrase) `hv_test_pw`'dir.

4. **Uygulama** şu komutla açılır (adım 11 hariç İngilizce; adım 13'ün hata ayıklama komutları
   yalnızca debug derlemede vardır, bu yüzden `--release` **kullanmayın**):

   ```
   cargo run -p hacivat -- --lang en
   ```

   Geliştirme derlemesi Keychain'e `hacivat-dev/<profil-uuid>/<yuva>` adıyla yazar (hesap
   `hacivat`). Yuvalar: `db-password`, `ssh-<n>-passphrase`, `ssh-<n>-password`; `<n>` atlamanın
   sırasıdır ve **0'dan başlar** (ilk atlama `ssh-0-…`). macOS'ta yorum alanı boştur; hangi uuid'nin
   hangi profile ait olduğunu `connections.toml` gösterir:

   ```
   grep -nE '^(id|name) =' "$HOME/Library/Application Support/dev.hacivat.Hacivat/connections.toml"
   ```

5. **Test sunucuları** (yalnızca demo içindir, gerçek parola değildir; `docker/m2a/README.md` ve
   `targets.toml`):
   - Veritabanı kullanıcısı `hv_test`, parola `hv_test_pw`, veritabanı `hv_test`. Salt okunur
     kullanıcı `hv_ro`, parolası aynı (`hv_test_pw`).
   - Portlar (Mac'ten, `127.0.0.1`): `pg17` 54317, `pg17-tls` 54327, `mysql84` 33084, `sshd-a` 2222.
     `sshd-b` dışarıya açık değildir; yalnızca `sshd-a` üzerinden erişilir.
   - SSH kullanıcısı `hv_ssh` (parolası `hv_ssh_pw`; bu demoda anahtar kullanılır).
   - **SSH üzerinden** veritabanı adresi kapsayıcı ağındaki addır: `pg17` port `5432`, `mysql84`
     port `3306`, bastion `sshd-b` port `22`.

6. Hatırlatmalar (M2a demosundan):
   - Gezginde bir satıra **çift tıklamak bağlanır**.
   - Modal bir katman (iletişim kutusu) açıkken genel kısayollar (⌘, ⌘K ⌘N) çalışmaz; bu tasarım
     gereğidir. Dil değişikliği kutu kapalıyken yapılır.
   - Bir şey farklıysa adım numarasıyla not alın; sonuçları sondaki tabloya işleyin.

---

## 1. `pg17-tls`, Auto: sertifika iletişim kutusu ve sabitleme (pin)

**Yap:** ⌘N. Motor PostgreSQL, Ad `pg17-tls`, Host `localhost`, Port `54327`, Kullanıcı `hv_test`,
Parola `hv_test_pw`, Veritabanı `hv_test`. Şifreleme (TLS) bölümünde Mod **Auto** kalsın.
"Save and connect" (⌘↵). Sertifika kutusu açılınca, parmak izini **başka bir terminalde** şununla
karşılaştırın:

```
docker exec m2a-pg17-tls sh -c 'openssl x509 -in "$PGDATA/server.crt" -noout -fingerprint -sha256'
```

Sonra "Pin this certificate".

**Nereye bak:** sertifika kutusu, sonra başlık çubuğundaki kapsül ve alt durum çubuğu.

**İyi görünüm:**
- Kutu başlığı **Certificate not trusted** / **Sertifikaya güvenilmiyor**. Gövde
  `localhost:54327 offered a certificate…` ile başlar. "Issued to" `localhost`.
- **SHA-256 fingerprint** / **SHA-256 parmak izi** iki satırda, 16'şar iki noktalı onaltılık bayt;
  `openssl` çıktısındaki `sha256 Fingerprint=` ile **birebir aynı**.
- Kutu **Cancel odaklı** açılır (Enter = Cancel). "Pin this certificate" / "Bu sertifikayı
  sabitle" bir düğmedir; varsayılan değildir.
- Sabitledikten sonra bağlanır. Kapsül ve durum çubuğu: **TLS 1.3 · pinned** / **TLS 1.3 ·
  sabitlendi**. "unencrypted" parçası yoktur.
- `connections.toml` içinde `tls.pins = [{ endpoint = "localhost:54327", sha256 = "…" }]` satırı
  vardır (küçük harf onaltılık; parmak iziyle aynı baytlar).

**Geçti:** parmak izi eşleşti, Cancel varsayılandı, kapsül "pinned" diyor.

**Ön doğrulama (kapı ajanı):** Docker `tls_auto_pin_and_verify_ca` (pg17-tls ve mysql84, doğrudan
ve SSH üzerinden: bir kez incelenir, sabitlenir, başsız yeniden bağlanır) yeşil. Uygulama:
`certificate_question_opens_review_dialog`, `the_review_opens_on_cancel_and_enter_presses_the_focused_button`,
`copy_copies_exactly_the_fingerprint` (connections_tls), `untrusted_then_pin_then_capsule_shows_pinned`
(cross_surface_m2b). Parmak izi komutu çalıştırıldı ve sunucunun sunduğu sertifikayla aynı değeri
verdi (kapsayıcı ağından `openssl s_client -starttls postgres` ile karşılaştırıldı).

## 2. Sertifika değişti: sabitleme kesin durdurur (hard stop)

**Önemli düzeltme:** planın metni "`pg17-tls`'i yeniden başlatın, sertifikası yenilenir" diyor.
Bu doğru **değil**: `pg-tls/init.sh` sertifikayı yalnızca ilk `initdb` sırasında üretir ve
`docker restart` veri birimini korur, yani sertifika aynı kalır. Bunun yerine sertifikayı yerinde
yenileyin (kapsayıcı yeniden başlamaz, sunucu `pg_reload_conf()` ile yeni sertifikayı yükler):

```
docker exec -u postgres m2a-pg17-tls sh -c 'cd "$PGDATA" && openssl req -new -x509 -days 397 -nodes -newkey rsa:2048 -subj "/CN=localhost" -addext "basicConstraints=critical,CA:FALSE" -addext "extendedKeyUsage=serverAuth" -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" -keyout server.key -out server.crt 2>/dev/null && chmod 600 server.key && psql -U hv_test -d hv_test -Atc "select pg_reload_conf()"'
```

Çıktı `t` olmalı. Yeni parmak izi için adım 1'deki `openssl x509` komutunu tekrar çalıştırın.

**Yap:** Gezginde `pg17-tls` bağlıysa bağlantıyı kesin (satırı seçip ↵), sonra yeniden bağlanın
(çift tıklama).

**İyi görünüm:**
- Kesin durdurma kutusu: **The certificate of localhost:54327 changed** / **localhost:54327
  sertifikası değişti**. Gövde: "…Hacivat stopped before signing in." / "…Hacivat oturum açmadan
  önce durdu."
- İki parmak izi: **Pinned** / **Sabitlenen** (eski, adım 1'deki) ve **Now** / **Şimdiki** (yeni,
  komutun yeni çıktısı).
- **Cancel varsayılandır** (Enter = Cancel). Önce Enter'a basın: kutu kapanır, bağlanmaz, hiçbir
  şey değişmez.
- Yeniden bağlanın, bu kez **Replace pin** / **Sabitlemeyi değiştir**: bağlanır, bildirim
  "Pinned the new certificate of localhost" / "localhost sunucusunun yeni sertifikası sabitlendi".
  Kapsül yine **TLS 1.3 · pinned**. `connections.toml`'daki `sha256` yeni parmak izidir; eskisi
  yoktur.

**Geçti:** iki parmak izi doğru, Enter iptal etti, Replace pin bağlandı.

**Ön doğrulama:** yenileme komutu kapı ajanı tarafından çalıştırıldı: sunulan parmak izi
`DD:1C:C4:…:FF:7D`'den `63:A4:A2:…:EE:75`'e değişti, kapsayıcı `healthy` kaldı ve ardından tam Docker
seti yine 99/99 geçti. Uygulama: `pin_changed_dialog_has_cancel_as_default`,
`replace_pin_from_error_writes_replace_pin`, `a_stale_replace_pin_changes_nothing` (connections_tls),
`replace_pin_completes_from_the_keyboard` (cross_surface_m2b). Çekirdek:
`rotated_certificate_is_pin_mismatch_with_both_fingerprints` (hacivat-net).

## 3. `mysql84`, Verify CA, kapsayıcının CA'sı (E8)

**Yap:** CA dosyasının tam yolunu depo kökünden yazdırın: `echo "$PWD/target/docker/mysql84-ca.pem"`.
⌘N. Motor MySQL, Ad `mysql84-ca`, Host `localhost`, Port `33084`, Kullanıcı `hv_test`, Parola
`hv_test_pw`, Veritabanı `hv_test`. Şifreleme bölümünde Mod **Verify CA** / **CA'yı doğrula**;
**CA certificate** / **CA sertifikası** alanına yazdırdığınız yolu yapıştırın (ya da dosya
seçicide ⌘⇧G ile yolu yazın). "Save and connect".

**İyi görünüm:**
- Hiçbir soru çıkmaz; bağlanır. Kapsül: **TLS 1.3 · CA verified** / **TLS 1.3 · CA doğrulandı**
  (zincir doğrulandı, ana bilgisayar adı değil; MySQL'in kendi ürettiği sertifikada ad yoktur).
- Verify CA seçeneği MySQL için **etkindir** (M2a'da pasifti).

**Geçti:** soru yok, "CA verified".

**Ön doğrulama:** Docker `tls_auto_pin_and_verify_ca` (mysql84 VerifyCa → `CaOnly`, doğrudan ve
SSH üzerinden); `mysql_verify_ca_enabled` (connections_tls). CA dosyası yerinde
(`CN=MySQL_Server_8.4.13_Auto_Generated_CA_Certificate`).

## 4. Genel ağda şifresiz sunucu sorusu

Yerel Docker matrisinde bu soruya **ulaşılamaz**: her hedef `127.0.0.1` (loopback), soru ise
yalnızca genel (public) bir adreste TLS sunmayan sunucuda çıkar. Planın kuralı (Q39) gereği
`demo-fakes` gibi bir özellik derlenmedi.

**Yap:** Şu ekran görüntülerine bakın (bölüm "Ekran görüntüleri", `2-m2b-new/`):
`plaintext-question-*` (her tema ve dil) ve `conn-error-tls-not-offered-*` ("Connect through
SSH…" düzeltmesi).

**İyi görünüm:** başlık "`<host>` doesn't offer encryption" / "`<host>` şifreleme sunmuyor";
varsayılan **Use an SSH tunnel…** / **SSH tüneli kullan…**; ikinci düğme "Connect without
encryption"; altında "Don't ask again for this connection" seçeneği.

**Ön doğrulama:** `plaintext_question_dialog`, `enter_uses_an_ssh_tunnel_and_opens_the_ssh_section`,
`a_host_or_port_change_clears_allow_public_plaintext` (connections_tls);
`use_an_ssh_tunnel_answer_end_to_end`, `connect_through_ssh_fix_end_to_end` (cross_surface_m2b).

## 5. SSH: `~/.ssh/config`'den içe aktar, yeni sunucu anahtarı, paylaşılan oturum

**Yap:**
1. SSH oturum sayısının taban değerini alın (bastion'daki kurulu SSH bağlantıları):

   ```
   docker exec m2a-sshd-a ss -Htn state established '( sport = :22 )' | wc -l
   ```

   Şu an `0` olmalı.
2. ⌘N. Motor PostgreSQL, Ad `pg17-ssh`, Host `pg17`, Port `5432`, Kullanıcı `hv_test`, Parola
   `hv_test_pw`, Veritabanı `hv_test`.
3. **SSH tunnel** / **SSH tüneli** bölümünü açın, **Connect through an SSH tunnel** / **SSH
   tüneli üzerinden bağlan** kutusunu işaretleyin, **Import from ~/.ssh/config…** / **~/.ssh/config'den
   içe aktar…** düğmesine basın ve listeden `hv-sshd-a`'yı seçin.
4. "Save and connect". Ortam sorusu çıkarsa **Dev** seçin (`pg17` bir ad, `localhost` değil).
5. Yeni sunucu anahtarı kutusunda parmak izini başka bir terminalde karşılaştırın:

   ```
   ssh-keygen -lf target/docker/ssh-host-sshd-a.pub
   ```

   Sonra **Trust and connect** / **Güven ve bağlan**.
6. İkinci profil: ⌘N. Motor MySQL, Ad `mysql84-ssh`, Host `mysql84`, Port `3306`, Kullanıcı
   `hv_test`, Parola `hv_test_pw`, Veritabanı `hv_test`; Şifreleme **Verify CA** ve adım 3'teki CA
   yolu; SSH bölümünde yine **Import… → `hv-sshd-a`**. "Save and connect".
7. İki bağlantı da açıkken oturum sayısını tekrar alın (1. adımdaki komut) ve toplam kurulu TCP
   bağlantısını da görün:

   ```
   docker exec m2a-sshd-a ss -tn | grep -c ESTAB
   ```

**İyi görünüm:**
- İçe aktarma bildirimi: "Imported hv-sshd-a from ~/.ssh/config: host, user, key and jump host."
  / "hv-sshd-a, ~/.ssh/config'den içe aktarıldı: adres, kullanıcı, anahtar ve ara sunucu."
  Alanlar: SSH host `127.0.0.1`, port `2222`, SSH user `hv_ssh`, Key file `~/.ssh/hv-demo-ed25519`.
  (Bildirim her zaman dört öğeyi sayar; içe aktarılana göre değişsin mi, açık PO sorusu, T20 M3.)
- Bağlanırken kapsülün adım adı önce **SSH** olur; **Resolve adımı yoktur** (veritabanı adı
  bastion'da çözülür, Mac'te DNS sorgusu yapılmaz). ⌘T ile sınarsanız izleme (trail) satırları
  `SSH, Connect, Sign in, Check server` sırasıyla akar.
- Yeni anahtar kutusu: **First connection to 127.0.0.1** / **127.0.0.1 ile ilk bağlantı**, gövde
  "This Mac hasn't connected to this SSH server before…". **Host key** satırı `SHA256:…` ile
  başlar ve `ssh-keygen -lf` çıktısındaki değerle **aynıdır**. Kutu Cancel odaklı açılır.
- Kapsül ve durum çubuğu: **SSH via 127.0.0.1** / **127.0.0.1 üzerinden SSH** ve gecikme (ms).
  `pg17`'nin TLS'i olmadığı için "unencrypted" parçası da görünür (veritabanı ayağı tünelin
  içindedir; rozet yine de kalıcıdır, B7). Gezgin satırında yol (route) simgesi vardır.
- MySQL profilinde **anahtar sorusu çıkmaz** (`sshd-a` artık biliniyor); kapsül **TLS 1.3 · CA
  verified** ve **SSH via 127.0.0.1**.
- 7. adımda **SSH oturum sayısı `1`**: iki profil tek bir SSH oturumunu paylaşır. Toplam `ESTAB`
  sayısı `3` civarıdır (1 SSH bağlantısı + bastion'dan `pg17:5432` ve `mysql84:3306`'ya iki
  yönlendirme). Paylaşım olmasaydı oturum sayısı 2 olurdu.

**Geçti:** parmak izi eşleşti, kapsül "SSH via", oturum sayısı 1.

Not: Mac'inizin ssh-agent'ında çok anahtar varsa ve bağlantı "Too many authentication failures"
gibi bir hatayla düşerse uygulamayı ajan olmadan açın: `SSH_AUTH_SOCK= cargo run -p hacivat -- --lang en`.

**Ön doğrulama:** komutlar çalıştırıldı (SSH oturum sayısı ve `ESTAB` = 0; yayınlanan anahtarın
parmak izi `ssh-keyscan` ile alınan canlı anahtarla aynı). Docker `connect_matrix_direct_and_via_ssh`
(her TCP hedefi doğrudan ve `sshd-a` üzerinden; izleme `Ssh, Connect, [Tls,] SignIn, CheckServer`,
PostgreSQL kanalın akışında, MySQL yönlendirici üzerinden), `two_profiles_share_one_ssh_session`
yeşil. Uygulama: `import_menu_lists_aliases_and_disables_proxycommand`, `import_fills_fields_and_shows_notice`
(connections_ssh_form), `new_host_key_prompt` (connections_prompts),
`ssh_connect_with_new_host_key_then_capsule_via_bastion` (cross_surface_m2b).

## 6. Atlama zinciri `sshd-a` → `sshd-b` → `pg17`, şifreli anahtar, "Forever"

**Yap:** ⌘N. Motor PostgreSQL, Ad `pg17-chain`, Host `pg17`, Port `5432`, Kullanıcı `hv_test`,
Parola `hv_test_pw`, Veritabanı `hv_test`. SSH bölümü (içe aktarmadan, elle):
- SSH host `sshd-b`, port `22`, SSH user `hv_ssh`, kimlik doğrulama **Key file** / **Anahtar
  dosyası**, Key file `~/.ssh/hv-demo-ed25519-enc`, Passphrase **boş** bırakın.
- **Add jump host** / **Ara sunucu ekle**: `127.0.0.1`, port `2222`, kullanıcı `hv_ssh`.
- SSH sırlarının **Remember** seçimi **Forever (Keychain)** / **Kalıcı (Anahtar Zinciri)**.

"Save and connect" (ortam: Dev). Çıkan sorulara cevap: `sshd-b` için yeni anahtar kutusu
(parmak izi: `ssh-keygen -lf target/docker/ssh-host-sshd-b.pub`) → Trust; iki parola ifadesi
sorusuna `hv_test_pw`, her birinde Remember **Forever**.

Sonra bağlantıyı kesin ve yeniden bağlanın.

**İyi görünüm:**
- SSH bölümündeki yol (route) satırı beş durak gösterir: This Mac → 127.0.0.1 (ara sunucu) →
  sshd-b (bastion) → pg17.
- **İki** parola ifadesi sorusu: başlık **Passphrase for ~/.ssh/hv-demo-ed25519-enc** /
  **~/.ssh/hv-demo-ed25519-enc için parola ifadesi**; alt satırda atlama adı ve sırası:
  "127.0.0.1, hop 1 of 2" / "127.0.0.1, 2 atlamadan 1." ve "sshd-b, hop 2 of 2" / "sshd-b, 2
  atlamadan 2.". Altta "Applies to the saved SSH passwords and passphrases of every hop of this
  connection."
- Bağlanır; kapsül **SSH via sshd-b**.
- Keychain Access'te `hacivat-dev` arayın: bu profilin uuid'siyle `…/ssh-0-passphrase` ve
  `…/ssh-1-passphrase` girişleri vardır.
- Kesip yeniden bağlanınca **hiçbir soru çıkmaz**.

**Geçti:** iki soru doğru atlamayı adlandırdı, yeniden bağlanma sorusuz.

**Ön doğrulama:** Docker `connect_matrix_direct_and_via_ssh` pg17 ve mysql84'ü `sshd-a` → `sshd-b`
zincirinden de bağlar (ilk atlama şifreli anahtarın parola ifadesiyle, ikincisi parolayla). Uygulama:
`ssh_secret_prompt_names_the_hop`, `ssh_secret_prompt_answers_the_chosen_remember`,
`ssh_passphrase_prompt_names_the_key_and_empty_enter_sends_nothing` (connections_prompts),
`ssh_secrets_follow_their_hop_when_hops_move` (connections_prompts).

## 7. Sunucu anahtarı değişti (Hacivat deposu)

**Yap:** `sshd-a`'nın anahtarını değiştirin:

```
docker exec m2a-sshd-a /regen-hostkey.sh
ssh-keygen -lf target/docker/ssh-host-sshd-a.pub   # yeni parmak izi
```

**Önce `sshd-a` üzerinden giden her bağlantıyı kesin** (`pg17-ssh`, `mysql84-ssh`, `pg17-chain`) ve
**en az 35 saniye bekleyin**. Açık bir bağlantı kaldıkça SSH oturumu paylaşılır ve yeni bağlantı
o oturumu kullanır, yani sunucu anahtarı yeniden sorulmaz; son bağlantı kapandıktan sonra tünel
30 s daha açık kalır (B11). Oturum sayısı `0` olmalı:

```
docker exec m2a-sshd-a ss -Htn state established '( sport = :22 )' | wc -l
```

Sonra `pg17-ssh`'a yeniden bağlanın.

**İyi görünüm:**
- Kesin durdurma kutusu: **The host key of 127.0.0.1 changed** / **127.0.0.1 sunucusunun anahtarı
  değişti**; gövde "…and also when someone is intercepting the connection. Hacivat stopped before
  signing in."
- Eski ve yeni anahtar (yeni olan, komutun çıktısıyla aynı). **Stored in** / **Kayıtlı olduğu
  yer**: **Hacivat's known hosts** / **Hacivat'ın bilinen sunucuları**.
- **Enter = Cancel**: önce Enter'a basın, kutu kapanır, bağlanmaz.
- Yeniden bağlanın; **Remove the old key and trust the new one** / **Eski anahtarı sil, yenisine
  güven**: bağlanır, bildirim "Trusted the new key of 127.0.0.1". `~/.ssh/known_hosts` dosyanız
  **değişmez** (Hacivat yazmaz).

**Geçti:** kutu Hacivat deposunu söyledi, Enter iptal etti, değiştirme bağlandı.

**Ön doğrulama:** komut kapı ajanı tarafından çalıştırıldı: yayınlanan ve canlı anahtar
`SHA256:95lOZ…`'den `SHA256:NuSJA…`'ya değişti, kapsayıcı `healthy` kaldı ve tam Docker seti yine
99/99 geçti. Uygulama: `host_key_changed_offers_replace_only_for_hacivat_store`,
`host_key_changed_enter_cancels`, `host_key_replace_rewrites_only_hacivats_store`
(connections_prompts).

## 8. Salt okunur: sunucu rolüyle zorunlu ve korumalı

**Yap (A):** ⌘N. Motor PostgreSQL, Ad `pg17-ro`, Host `localhost`, Port `54317`, Kullanıcı
**`hv_ro`**, Parola `hv_test_pw`, Veritabanı `hv_test`, **Read-only** / **Salt okunur** işaretli.
"Save and connect". Kapsüldeki erişim parçasına tıklayın.

**Yap (B):** Aynısını Ad `pg17-ro-admin`, Kullanıcı **`hv_test`** ile yapın. Erişim parçasına tıklayın,
"Copy SQL"e basın ve panoyu bir metin düzenleyiciye yapıştırın.

**İyi görünüm:**
- (A) Kapsül: **read-only (enforced by server role)** / **salt okunur (sunucu rolüyle zorunlu)**.
  Açıklama: "Read-only, enforced by the server" / "Salt okunur, sunucu tarafından zorunlu".
- (B) Kapsül: **read-only (guarded)** / **salt okunur (korumalı)**. Açıklama başlığı "Read-only is
  guarded, not enforced" / "Salt okunurluk korumalı, zorunlu değil"; bulgular listesi (süper
  kullanıcı vb.) ve SQL parçası: `CREATE ROLE <role> LOGIN PASSWORD '<password>';`,
  `GRANT pg_read_all_data TO <role>;` ve üç `-- optional:` yorum satırı (`REVOKE TEMP`,
  `REVOKE CREATE ON SCHEMA public`, `REVOKE EXECUTE ON FUNCTION pg_logical_emit_message(…)`).
  "Copy SQL" panoya **tam olarak** bu SQL'i koyar.
- Kilit simgesi yalnızca üretim ortamında kırmızıdır (DESIGN.md).

**Geçti:** iki etiket doğru, açıklama klavyeyle de açılıyor (Tab ile erişim parçası, Space).

**Ön doğrulama:** `pg17`'de `hv_ro` matris parolasıyla giriş yapıyor, süper kullanıcı değil ve
`pg_logical_emit_message`'ın iki biçiminde de EXECUTE yetkisi yok (F1 için paylaşılan `hv_test`
veritabanlarına `REVOKE EXECUTE` uygulanmış). Docker `docker_readonly` 10/10 (hv_ro pg13–18
zorunlu). Uygulama: `read_only_labels_and_remediation_popover`, `copy_sql_copies_the_remediation_exactly`,
`the_explanation_is_keyboard_reachable` (connections_live_m2b).

## 9. Yeniden bağlanma: sunucu oturumu sonlandırılır

**Önemli düzeltme:** M2b'nin varsayılanlarıyla (B18) doğrudan bir bağlantıda keep-alive **kapalıdır**
(yalnızca SSH 60 s alır) ve M2b'de henüz sorgu konsolu yoktur. Bu yüzden boşta bir bağlantının
arka ucu sonlandırılınca **kendiliğinden hiçbir şey olmaz**; kayıp bir sonraki istekte görülür.
Planın "kapsül Reconnecting gösterir" beklentisi için bağlantının bir istek göndermesi gerekir.
İki yol var, birini seçin:
- **(a)** `pg17-tls` profilini düzenleyin (⌘E), **Advanced** bölümünde **Keep-alive every** /
  **30 s** seçin, kaydedin ve bağlanın. Sonlandırmadan sonra en geç 30 s bekleyin.
- **(b)** Sonlandırmadan sonra ⌘K ile paleti açın ve **Debug: run SELECT pg_sleep(30) on the current
  connection** komutunu çalıştırın (adım 13); bu istek kaybı hemen bulur.

**Yap:** `pg17-tls` bağlıyken (ya da herhangi bir pg17 profili), yönetici `psql`'den varsayılan
oturumun arka ucunu sonlandırın:

```
docker exec m2a-pg17-tls psql -U hv_test -d hv_test -c "select pid, application_name from pg_stat_activity where application_name = 'Hacivat'"
docker exec m2a-pg17-tls psql -U hv_test -d hv_test -c "select pg_terminate_backend(pid) from pg_stat_activity where application_name = 'Hacivat' and pid <> pg_backend_pid()"
```

**İyi görünüm:**
- Kapsül kısa bir süre **Reconnecting · attempt 1** / **Yeniden bağlanıyor · 1. deneme** gösterir
  (ilk deneme beklemesizdir; çok kısa görünebilir), sonra yeniden **Connected** ve **TLS 1.3 ·
  pinned**.
- Bildirim: **Reconnected to pg17-tls** / **pg17-tls ile yeniden bağlanıldı**, gövde "The server
  session ended, so Hacivat opened a new one." / "Sunucu oturumu sona erdi; Hacivat yeni bir oturum
  açtı." Açık işlem olmadığı için "rolled back" satırı **yoktur**.
- (b) yolunda deneme ifadesi kopan bağlantıya çarpar ve **The demo statement failed.** / **Deneme
  ifadesi başarısız oldu.** ile biter; bağlantı yeniden kurulur ama kullanıcı ifadesi **tekrar
  çalıştırılmaz** (B19).

**Geçti:** Reconnecting → Connected, "Reconnected to" bildirimi, geri alma satırı yok.

**Ön doğrulama:** Docker `a_real_server_ending_the_session_is_reconnected_with_session_lost` (pg17 ve
mysql84: arka uç sonlandırılır, bir istek kaybı bulur, profil yeniden bağlanır, `SessionLost` geri
alma iddia etmez), `terminated_backend_with_open_tx_reports_lost_tx` (açık işlemde: `Lost`, yeniden
oynatma yok), `server_outage_reconnects_within_backoff` yeşil. Uygulama:
`a_real_reconnect_counts_down_and_ends_with_the_reconnected_toast`,
`a_reconnected_session_claims_no_rollback` (connections_live_m2b).

## 10. Boşta kalınca bağlantıyı kesme

**Yap:** Uygulamadan çıkın (⌘Q; `connections.toml` çalışırken izlenmez). `connections.toml`'u açın
ve `pg17-ro` profilinin `[[connection]]` bloğuna şu satırı ekleyin (`name = "pg17-ro"` satırının
altına):

```
idle_disconnect_s = 60
```

Uygulamayı yeniden açın, `pg17-ro`'ya bağlanın ve **hiçbir şeye dokunmadan** 61 saniye bekleyin.

**İyi görünüm:**
- Bildirim: **Disconnected pg17-ro after 1 min idle. It reconnects when you use it.** /
  **pg17-ro bağlantısı 1 dk boşta kaldıktan sonra kesildi. Kullandığınızda yeniden bağlanır.**
- Kapsül: **Disconnected (idle)** / **Bağlantı kesik (boşta)**. Kapsüle tıklamak yeniden bağlar.

**Geçti:** 60–62 s arasında bildirim, tıklayınca yeniden bağlanıyor.

**Ön doğrulama:** `idle_disconnected_toast_once`, `an_idle_threshold_in_minutes_says_minutes`,
`an_idle_connection_says_so_everywhere_and_reconnects_from_the_capsule` (connections_live_m2b);
çekirdek boşta kesme ve keep-alive testleri (`liveness`, hacivat-session) yeşil.

## 11. EN ↔ TR: yeni iletişim kutularında İngilizce kalıntı yok

Modal bir kutu açıkken dil değiştirilemez. Bu yüzden kutuları Türkçe arayüzde yeniden tetikleyin.

**Yap:** Uygulamadan çıkıp `cargo run -p hacivat -- --lang tr` ile açın ve sırayla:
1. Sertifika incelemesi: yeni bir profil `pg17-tls-tr` (adım 1'in değerleri) → kutu; **İptal**.
2. Sabitleme değişti: adım 2'deki yenileme komutunu tekrar çalıştırın, `pg17-tls`'e bağlanın →
   kutu; **İptal** (sonra isterseniz değiştirin).
3. Sunucu anahtarı değişti: `sshd-a` üzerinden giden bağlantıları kesip 35 s bekleyin (adım 7),
   `docker exec m2a-sshd-a /regen-hostkey.sh`, `pg17-ssh`'a bağlanın → kutu; **İptal**.
4. Parola ifadesi: `pg17-chain` profilinde SSH **Remember** seçimini **Hiçbir zaman** yapın,
   kaydedip bağlanın → iki soru; değerleri girin.
5. Salt okunur açıklaması: `pg17-ro-admin`'e bağlanıp erişim parçasına tıklayın.
6. Bildirimler: adım 9'u Türkçe tekrarlayın.

**İyi görünüm:** her etiket, düğme, ipucu ve bildirim Türkçedir; **İngilizce kalıntı yoktur**.
Sunucudan gelen metin (sertifika adları, parmak izleri, SQL) olduğu gibi kalır. Türkçe metinlerin
yazımı ve anlamı PO inceleme listesindedir (`m2b.md`, bölüm 9.3).

**Ön doğrulama:** ekran görüntüsü düzeneğinin (a) denetimi: `en-XA` ve `tr-XT` sahte dillerinde
her M2b yüzeyindeki her Fluent etiketi köşeli parantezlidir, yani kodda sabit İngilizce metin yok
(1012 görüntü, 0 ihlal). `turkish_fits_the_review_and_the_question`, `turkish_read_only_texts_fit`,
`turkish_lost_list_uses_ve` (uygulama testleri).

## 12. SSH üzerinden bağlıyken çıkış

**Yap:** `pg17-chain` ve `pg17-ssh` bağlıyken ⌘Q.

**İyi görünüm:**
- Uygulama **2 saniye içinde** kapanır; `cargo run` hemen döner.
- Bastion'da asılı oturum kalmaz:

  ```
  docker exec m2a-sshd-a ss -Htn state established '( sport = :22 )' | wc -l   # 0
  ```

**Geçti:** ≤ 2 s ve 0 oturum.

**Ön doğrulama:** `quit_closes_every_connection`, `quit_during_connect_cancels_within_budget`
(connections_live), `a_forever_save_into_an_unlocked_vault_drains_at_quit` (connections_prompts);
Task 21 incelemesinde SSH ile çıkış 1,2 s ölçüldü (P15 bütçesi 2 s).

## 13. İptal (debug derleme)

**Yap:** `pg17-tls`'e bağlıyken ⌘K, **Debug: run SELECT pg_sleep(30) on the current connection** /
**Hata ayıklama: geçerli bağlantıda SELECT pg_sleep(30) çalıştır**. Birkaç saniye sonra ⌘K,
**Debug: cancel the demo statement** / **Hata ayıklama: deneme ifadesini iptal et**. Ardından ilk
komutu bir kez daha çalıştırıp hemen iptal edin.

**İyi görünüm:**
- İptalden sonra **1 saniye içinde** bildirim: **The demo statement was cancelled.** / **Deneme
  ifadesi iptal edildi.**
- Oturum kullanılabilir kalır: ikinci deneme de çalışır ve iptal edilir; kapsül "Connected" kalır.
- `--release` derlemede bu komutlar paletinde **yoktur**.

**Geçti:** ≤ 1 s, oturum kullanılabilir.

**Ön doğrulama:** Docker `cancel_matrix` (pg17, mysql84, mariadb114 doğrudan ve SSH üzerinden,
SQLite: her biri 1 s içinde; sonraki ifade çalışır), `stale_cancel_never_hits_the_next_request`
(hacivat-session). Uygulama: `debug_cancel_command_cancels_the_demo_sleep`,
`release_builds_have_no_cancel_demo` (connections_live_m2b).

---

## Demo sonrası zorunlu kontroller

1. **E9, diskte gizli yok.** Uygulama kapalıyken hiçbir yerde demo parolası ya da parola ifadesi
   kalmamalı (`known_hosts` ve günlükler bu klasörlerin içindedir):

   ```
   grep -r -e hv_test_pw -e hv_ssh_pw "$HOME/Library/Application Support/dev.hacivat.Hacivat" "$HOME/Library/Caches/dev.hacivat.Hacivat"
   ```

   Beklenen: çıktı **boş** (çıkış kodu 1).
2. **Idle RSS, bir doğrudan ve bir SSH bağlantısıyla (E13).** Uygulamayı **release** derlemeyle açın
   (`cargo run -p hacivat --release -- --lang en`), `pg17-tls` (doğrudan) ve `pg17-ssh` (SSH)
   profillerine bağlanın, 10 saniye dokunmadan bekleyin ve ölçün:

   ```
   ps -o rss= -p "$(pgrep -n -x hacivat)" | awk '{printf "%.0f MiB\n", $1/1024}'
   ```

   Değeri sonuç tablosuna yazın (bütçe: bağlantısız 200 MB; bağlantılı değer yalnızca kaydedilir).
3. Keychain Access'te `hacivat-dev` arayın: silinen profillerin girişi kalmamalı.

## Temizlik

```
sed -i '' '/^# >>> hacivat-demo$/,/^# <<< hacivat-demo$/d' ~/.ssh/config
sed -i '' '1{/^$/d;}' ~/.ssh/config
diff ~/.ssh/config ~/.ssh/config.hv-demo.bak && rm ~/.ssh/config.hv-demo.bak   # fark yoksa yedeği sil
rm ~/.ssh/hv-demo-ed25519 ~/.ssh/hv-demo-ed25519-enc
```

`~/.ssh/config` demodan önce yoksa, ilk `sed` boş bir dosya bırakır; silebilirsiniz. M2a'dan kalan
uygulama klasörlerini geri almak isterseniz `.m2a-bak` adlarını geri çevirin (önce M2b demo
klasörlerini silin ya da kenara taşıyın).

## Ekran görüntüleri

Kapı ajanı son görüntüleri `<ana çalışma kopyası>/target/shots-review-m2b/` altına koydu:
- `1-m2a-changed/`: m2a ucundaki görüntüden farklı olan M2a yüzeyleri, `__ONCE` (m2a) ve `__SIMDI`
  (m2b) çiftleri;
- `2-m2b-new/`: M2b'nin yeni yüzeyleri (704 görüntü);
- `_render-m2b/`: kapı ucunun 1012 görüntüsünün tamamı.

108 değişen çiftin nedenleri `m2b.md` bölüm 7'dedir. Onaydan sonra `_render-m2b/` içindeki macOS
PNG'leri `design/baselines/m2b/macos/` altına kopyalanır (kapı ajanı bunu yapmadı). Türkçe metinleri bu görüntülerde de gözden geçirin.

## Sonuç tablosu (PO doldurur)

| Adım | Sonuç (GEÇTİ / KALDI) | Not |
|---|---|---|
| 1 pg17-tls sertifika, pin, "pinned" | | |
| 2 Sertifika değişti, Cancel varsayılan, Replace pin | | |
| 3 mysql84 Verify CA | | |
| 4 Şifresiz genel sunucu (görüntülerden) | | |
| 5 SSH içe aktarma, yeni anahtar, paylaşılan oturum (sayı: __) | | |
| 6 Atlama zinciri, iki parola ifadesi, Forever | | |
| 7 Sunucu anahtarı değişti | | |
| 8 Salt okunur zorunlu / korumalı | | |
| 9 Yeniden bağlanma ve bildirim | | |
| 10 Boşta kesme | | |
| 11 EN ↔ TR | | |
| 12 SSH ile çıkış ≤ 2 s | | |
| 13 İptal ≤ 1 s | | |
| E9 grep boş | | |
| E13 RSS (1 doğrudan + 1 SSH): ___ MiB | | |
