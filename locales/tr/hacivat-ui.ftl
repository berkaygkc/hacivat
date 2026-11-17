### The Hacivat desktop shell (prefix `hv-`).
### Append-only: each task adds a block headed `## <task slug>`.

## task-13-shell

hv-titlebar-palette = Ara veya komut çalıştır
# Review: PO
hv-capsule-no-connection = Bağlantı yok
hv-capsule-env-production = PROD
hv-capsule-access-read-only = salt okunur
hv-capsule-path-direct = Doğrudan
hv-panel-explorer = GEZGİN
hv-panel-connection = BAĞLANTI
hv-explorer-filter = Bağlantılarda ara
# Review: PO
hv-explorer-empty = Henüz bağlantı yok
# Review: PO
hv-tab-console-untitled = Konsol { $n }
# Review: PO
hv-status-no-connection = Bağlantı yok
# Review: PO
hv-theme-dark = Koyu
# Review: PO
hv-theme-light = Açık
# Review: PO
hv-editor-empty = SQL yazmaya başlamak için bir veritabanına bağlanın
# Review: PO
hv-results-empty = Sonuçları burada görmek için bir sorgu çalıştırın
# Review: PO
hv-status-pending-chord = Sonraki tuş bekleniyor: { $chord }
# Review: PO
hv-settings-invalid-value = Bazı ayarlar okunamadı, varsayılanları kullanılıyor: { $keys }
# Review: PO
hv-settings-newer-version = Ayarlar dosyanız Hacivat'ın daha yeni bir sürümünden. Bu sürümün tanımadığı ayarlar korunur.
# Review: PO
hv-settings-file-quarantined = Ayarlar dosyanız okunamadı. { $file } olarak kaydedildi, varsayılanlar kullanılıyor.

## Task 16

# Review: PO
hv-cmd-workspace-command-palette = Komut paleti
hv-cmd-workspace-open-settings = Ayarları aç
# Review: PO
hv-cmd-workspace-new-console-tab = Yeni konsol sekmesi
# Review: PO
hv-cmd-workspace-close-tab = Sekmeyi kapat
# Review: PO
hv-cmd-workspace-next-tab = Sonraki sekme
# Review: PO
hv-cmd-workspace-prev-tab = Önceki sekme
# Review: PO
hv-cmd-workspace-split-right = Sağa böl
# Review: PO
hv-cmd-workspace-split-down = Aşağı böl
# Review: PO
hv-cmd-workspace-focus-next-pane = Sonraki bölmeye geç
# Review: PO
hv-cmd-workspace-toggle-explorer = Gezgini göster veya gizle
# Review: PO
hv-cmd-workspace-toggle-theme = Koyu ve açık tema arasında geç
# Review: PO
hv-cmd-workspace-next-language = Sonraki dile geç
# Review: PO
hv-cmd-workspace-reload-translations = Çevirileri yeniden yükle
hv-cmd-app-about = { -brand-hacivat } Hakkında
# Review: PO
hv-cmd-app-show-menu = Uygulama menüsünü göster
# Review: PO
hv-cmd-app-restart = Yeniden başlat
# Review: PO
hv-cmd-app-quit = Uygulamadan çık
# Review: PO
hv-cmd-explorer-focus-filter = Bağlantı filtresine odaklan
hv-menu-app = { -brand-hacivat }
hv-menu-view = Görünüm
hv-menu-window = Pencere
hv-menu-help = Yardım
hv-about-title = { -brand-hacivat } Hakkında
hv-about-version = Sürüm { $version }
hv-toast-translations-reloaded = Çeviriler yeniden yüklendi

## Task 17

# Review: PO
hv-settings-title = Ayarlar
# Review: PO
hv-settings-nav-general = Genel
# Review: PO
hv-settings-nav-appearance = Görünüm
# Review: PO
hv-settings-nav-keyboard = Klavye
# Review: PO
hv-settings-search = Ayarlarda ara
# Review: PO
hv-settings-no-results = Eşleşen ayar yok
# Review: PO
hv-settings-language = Dil
# Review: PO
hv-settings-regional-format = Bölgesel biçim
# Review: PO
hv-settings-theme = Tema
# Review: PO
hv-settings-text-rendering = Metin işleme
# Review: PO
hv-settings-keymap = Klavye kısayolları
# Review: PO
hv-settings-system = Sistem
# Review: PO
hv-settings-text-rendering-crisp = Net
# Review: PO
hv-settings-text-rendering-standard = macOS standardı
# Review: PO
hv-settings-restart-note = Yeniden başlatınca uygulanır
# Review: PO
hv-settings-restart-now = Şimdi yeniden başlat
# Review: PO
hv-settings-restart-title = Hacivat yeniden başlatılsın mı?
# Review: PO
hv-settings-restart-body = Yeni metin işleme için Hacivat kapanıp yeniden açılır. Ayarlarınız ve düzeniniz önce kaydedilir.
# Review: PO
hv-settings-restart-later = Sonra
# Review: PO
hv-settings-regional-preview = { NUMBER($number) } · { DATETIME($when, dateStyle: "short", timeStyle: "short") }
# Review: PO
hv-settings-keymap-collisions = Bu klavye düzeninde şu kısayollar bunun yerine bir karakter yazar: { $chords }
# Review: PO
hv-settings-section-general = GENEL
# Review: PO
hv-settings-section-appearance = GÖRÜNÜM
# Review: PO
hv-settings-section-keyboard = KLAVYE
# Review: PO
hv-settings-keymap-hacivat = { -brand-hacivat }
# Review: PO
hv-settings-keymap-datagrip = DataGrip
# Review: PO
hv-settings-keymap-collision = { $chord } → { $char }

## m2a-t01-menu

# Review: PO
hv-menu-file = Dosya

## m2a-t12-profiles

# Review: PO
hv-conn-keyring-unavailable = Parolalar şu an sistemin anahtar deposuna kaydedilemiyor. { -brand-hacivat } onları kapanana kadar saklar.
# Review: PO
hv-conn-store-read-only = Bağlantılar dosyanız daha yeni bir { -brand-hacivat } sürümünden. Bu sürümde bağlantı eklenemez ya da değiştirilemez.
# Review: PO
hv-conn-file-quarantined = Bağlantılar dosyanız okunamadı. { $file } olarak kaydedildi ve hiçbir bağlantı yüklenmedi.
# Review: PO
hv-conn-file-entries-skipped = Bağlantılar dosyanızdaki { $count } bağlantı okunamadı ve atlandı.
# Review: PO
hv-conn-file-password-ignored = Bağlantılar dosyanızdaki bir parola yok sayıldı. { -brand-hacivat } parolaları o dosyada değil, sistemin anahtar deposunda saklar.
# Review: PO
hv-conn-toast-dup = { $name } olarak çoğaltıldı
# Review: PO
hv-conn-file-unreadable = { -brand-hacivat } bağlantılar dosyanızı okuyamıyor. Dosya olduğu gibi bırakıldı; okunabilene kadar bağlantılar değiştirilemez.
# Review: PO
hv-conn-file-values-ignored = Bağlantılar dosyanızdaki { $count } ayar okunamadı ve yok sayıldı.

## m2a-t18-explorer

# Review: PO
hv-cmd-connection-new = Yeni bağlantı
# Review: PO
hv-cmd-connection-paste-url = Kopyalanan URL'den yeni bağlantı
# Review: PO
hv-cmd-connection-toggle-connect = Bağlan ya da bağlantıyı kes
# Review: PO
hv-cmd-connection-edit = Bağlantıyı düzenle
# Review: PO
hv-cmd-connection-duplicate = Bağlantıyı çoğalt
# Review: PO
hv-cmd-connection-copy-url = Bağlantıyı URL olarak kopyala
# Review: PO
hv-cmd-connection-delete = Bağlantıyı sil
# Review: PO
hv-cmd-connection-context-menu = Bağlantı menüsünü göster
# Review: PO
hv-cmd-connection-import-pg-service = pg_service.conf içe aktar
# Review: PO
hv-cmd-connection-retry = Bağlantıyı yeniden dene
# Review: PO
hv-cmd-connection-explain-unencrypted = Şifrelemesiz bağlantıyı açıkla
# Review: PO
hv-cmd-workspace-focus-notification = En yeni bildirime git
# Review: PO
hv-explorer-empty-body = Bir PostgreSQL, MySQL, MariaDB ya da SQLite veritabanı ekleyin. Bir bağlantı URL'si yapıştırırsanız { -brand-hacivat } formu sizin için doldurur.
# Review: PO
hv-explorer-new = Yeni bağlantı
# Review: PO
hv-explorer-paste = URL yapıştır
# Review: PO
hv-explorer-import = pg_service.conf içe aktar…
# Review: PO
hv-explorer-group-production = ÜRETİM
# Review: PO
hv-explorer-group-staging = HAZIRLIK
# Review: PO
hv-explorer-group-test = TEST
# Review: PO
hv-explorer-group-dev = GELİŞTİRME
# Review: PO
hv-explorer-group-local = YEREL
# Review: PO
hv-explorer-group-unset = ORTAM YOK
# Review: PO
hv-conn-menu-connect = Bağlan
# Review: PO
hv-conn-menu-disconnect = Bağlantıyı kes
# Review: PO
hv-conn-menu-edit = Düzenle…
# Review: PO
hv-conn-menu-dup = Çoğalt
# Review: PO
hv-conn-menu-copy-url = URL olarak kopyala
# Review: PO
hv-conn-menu-delete = Sil…
# Review: PO
hv-conn-del-title = { $name } silinsin mi?
# Review: PO
hv-conn-del-body = Kayıtlı parolası Anahtar Zinciri'nden de silinir. Betikler ve sorgu geçmişi kalır. Bu işlem geri alınamaz.
# Review: PO
hv-conn-del-btn = Sil
# Review: PO
hv-conn-del-cancel = Vazgeç
# Review: PO
hv-conn-toast-deleted = { $name } silindi
# Review: PO
hv-conn-toast-cancelled = { $name } bağlantısı iptal edildi.
# Review: PO
hv-conn-sheet-none = Ayrıntılarını görmek için bir bağlantı seçin.
# Review: PO
hv-conn-kv-env = Ortam
# Review: PO
hv-conn-kv-engine = Motor
# Review: PO
hv-conn-kv-host = Adres
# Review: PO
hv-conn-kv-file = Dosya
# Review: PO
hv-conn-kv-user = Kullanıcı
# Review: PO
hv-conn-kv-access = Erişim
# Review: PO
hv-conn-kv-path = Yol
# Review: PO
hv-conn-kv-enc = Şifreleme
# Review: PO
hv-conn-kv-secret = Parola
# Review: PO
hv-conn-access-ro-guarded = salt okunur (korumalı)
# Review: PO
hv-conn-enc-verified = { $version } · doğrulandı
# Review: PO
hv-conn-enc-unverified = { $version } · doğrulanmadı
# Review: PO
hv-conn-enc-unencrypted = şifrelenmemiş
# Review: PO
hv-conn-enc-none-file = geçerli değil (dosya)
# Review: PO
hv-conn-secret-forever = Anahtar Zinciri · kalıcı
# Review: PO
hv-conn-secret-never = her seferinde sorar
# Review: PO
hv-conn-secret-session = bu oturum boyunca
# Review: PO
hv-conn-secret-restart = yeniden başlatana kadar
# Review: PO
hv-conn-secret-pgpass = ~/.pgpass dosyasından
# Review: PO
hv-conn-secret-none = yok
# Review: PO
hv-conn-unenc-title = Şifrelemesiz bağlanıldı
# Review: PO
hv-conn-unenc-body = { $host } TLS sunmuyor. Adres özel bir ağda olduğu için { -brand-hacivat } şifresiz bağlantıya izin verdi. Bağlantı şifresiz kaldıkça bu rozet görünür.
# Review: PO
hv-conn-unenc-fix = Kaldırmak için sunucuda TLS'i açın ya da SSH üzerinden bağlanın.
# Review: PO
hv-capsule-connecting = Bağlanıyor
# Review: PO
hv-capsule-failed = Bağlanılamadı
# Review: PO
hv-capsule-retry = Yeniden dene
# Review: PO
hv-capsule-clock = { NUMBER($seconds, minimumFractionDigits: 2) } sn
# Review: PO
hv-capsule-unsupported = desteklenmiyor
# Review: PO
hv-capsule-access-read-write = okuma-yazma
# Review: PO
hv-capsule-latency = { $ms } ms
# Review: PO
hv-status-env = { $env } · { $access }
# Review: PO
hv-home-title = Açık sorgu yok
# Review: PO
hv-home-hint = Gezgin'den bir bağlantı seçin ya da yeni bir tane ekleyin.
# Review: PO
hv-home-new = Yeni bağlantı
# Review: PO
hv-home-paste = Kopyalanan URL'den yeni bağlantı
# Review: PO
hv-home-connect = Seçili bağlantıya bağlan
# Review: PO
hv-conn-import-title = pg_service.conf'tan içe aktar
# Review: PO
hv-conn-import-body = İçe aktarılacak servisleri seçin. Her uzak servis için bir ortam gerekir.
# Review: PO
hv-conn-import-confirm = İçe aktar
# Review: PO
hv-conn-import-cancel = Vazgeç
# Review: PO
hv-conn-import-row = { $service } · { $target }
# Review: PO
hv-conn-import-empty = pg_service.conf içinde servis bulunamadı.
# Review: PO
hv-conn-import-unreadable = pg_service.conf okunamadı.
# Review: PO
hv-conn-import-password-ignored = Dosyayı başka kullanıcılar okuyabildiği için { $service } parolası yok sayıldı.
# Review: PO
hv-conn-import-done = { $count } bağlantı içe aktarıldı
## m2a-t20-errors

# Review: PO
hv-conn-test-running = Test ediliyor
# Review: PO
hv-conn-test-clock = { NUMBER($seconds, minimumFractionDigits: 2) } sn
# Review: PO
hv-conn-test-ok = { $ms } ms içinde bağlandı
# Review: PO
hv-conn-test-ok-sub = Hiçbir şey kaydedilmedi. Test kendi bağlantısını açtı ve kapattı.
# Review: PO
hv-conn-test-copy = Raporu kopyala
# Review: PO
hv-conn-diag-server = Sunucu
# Review: PO
hv-conn-diag-latency = Gecikme
# Review: PO
hv-conn-diag-tls = Şifreleme
# Review: PO
hv-conn-diag-cipher = Şifre takımı
# Review: PO
hv-conn-diag-auth = Oturum
# Review: PO
hv-conn-diag-warn = Uyarılar
# Review: PO
hv-conn-diag-none = Yok
# Review: PO
hv-conn-diag-server-v = { $product } { $version }
# Review: PO
hv-conn-diag-latency-v = { $ms } ms · 3 ölçümün ortancası
# Review: PO
hv-conn-diag-auth-v = { $user } · { $method ->
        [password] Parola
        [pgpass] pgpass
        [client_cert] İstemci sertifikası
       *[none] parolasız
    }
# Review: PO
hv-conn-diag-auth-method = { $method ->
        [password] Parola
        [pgpass] pgpass
        [client_cert] İstemci sertifikası
       *[none] Parolasız
    }
# Review: PO
hv-conn-diag-tls-v = TLS { $version } · { $state ->
        [full] doğrulandı
        [ca_only] CA doğrulandı
       *[no] doğrulanmadı
    }
# Review: PO
hv-conn-diag-tls-nov = TLS · { $state ->
        [full] doğrulandı
        [ca_only] CA doğrulandı
       *[no] doğrulanmadı
    }
# Review: PO
hv-conn-diag-tls-none = şifrelenmemiş
# Review: PO
hv-conn-diag-file = geçerli değil (dosya)
# Review: PO
hv-conn-diag-row = { $label }: { $value }
# Review: PO
hv-conn-step-resolve = Çözümle
# Review: PO
hv-conn-step-tcp = Bağlan
# Review: PO
hv-conn-step-tls = TLS
# Review: PO
hv-conn-step-auth = Oturum aç
# Review: PO
hv-conn-step-check = Sunucuyu denetle
# Review: PO
hv-conn-step-pgpass = pgpass
# Review: PO
hv-conn-step-open = Dosyayı aç
# Review: PO
hv-conn-step-ms = { $ms } ms
# Review: PO
hv-conn-step-list = Bağlantı adımları
# Review: PO
hv-conn-step-state-pending = başlamadı
# Review: PO
hv-conn-step-state-running = sürüyor
# Review: PO
hv-conn-step-state-done = tamam
# Review: PO
hv-conn-step-state-failed = başarısız
# Review: PO
hv-conn-step-state-skipped = atlandı
# Review: PO
hv-conn-warn-no-tls = Bağlantı şifrelenmemiş.
# Review: PO
hv-conn-warn-tls-not-verified = Bağlantı şifreli, ama sunucunun sertifikası doğrulanmadı.
# Review: PO
hv-conn-warn-not-tested = { -brand-hacivat } henüz { $version } sürümüyle test edilmedi.
# Review: PO
hv-conn-warn-unsupported = { $version } sürümü, desteklenen sürümlerden eski. Yine de bağlanmayı seçtiniz.
# Review: PO
hv-conn-warn-replica = Sunucu salt okunur bir kopya (replika).
# Review: PO
hv-conn-warn-server-read-only = Sunucu salt okunur kipte.
# Review: PO
hv-conn-warn-super-read-only = Sunucu süper salt okunur kipte: yöneticiler dahil kimse yazamaz.
# Review: PO
hv-conn-warn-low-wait-timeout = Sunucu boştaki bağlantıları { $seconds } sn sonra kapatır.
# Review: PO
hv-conn-warn-local-infile-on = Sunucu LOAD DATA LOCAL INFILE'a izin veriyor; bu Mac'ten dosya isteyebilir.
# Review: PO
hv-conn-warn-sqlite-file-not-writable = Veritabanı dosyasına yazılamıyor; salt okunur açılır.
# Review: PO
hv-conn-warn-sqlite-journal = Veritabanı { $mode } günlük kipini kullanıyor.
# Review: PO
hv-conn-err-title = { $name } bağlantısı kurulamadı
# Review: PO
hv-conn-err-attempt = 3 denemeden { $n }.
# Review: PO
hv-conn-err-new-pw = { $user } için parola
# Review: PO
hv-conn-err-tech = Teknik ayrıntılar
# Review: PO
hv-conn-err-stage = Hata adımı
# Review: PO
hv-conn-err-class = Sınıf
# Review: PO
hv-conn-err-code = Kod
# Review: PO
hv-conn-err-server = Sunucu mesajı
# Review: PO
hv-conn-err-elapsed-label = Geçen süre
# Review: PO
hv-conn-err-elapsed = { NUMBER($seconds, minimumFractionDigits: 3) } sn
# Review: PO
hv-conn-err-copy = Ayrıntıları kopyala (İngilizce)
# Review: PO
hv-conn-fix-reenter = Parolayı yeniden gir
# Review: PO
hv-conn-fix-edit = Bağlantıyı düzenle
# Review: PO
hv-conn-fix-retry = Yeniden dene
# Review: PO
hv-conn-fix-use-db = { $db } kullan
# Review: PO
hv-conn-fix-versions = Desteklenen sürümler
# Review: PO
hv-conn-fix-anyway = Yine de bağlan (desteksiz)
# Review: PO
hv-conn-fix-connect = Bağlan
# Review: PO
hv-conn-fix-close = Kapat
# Review: PO
hv-conn-fix-read-only = Bağlantılar dosyanız değiştirilemiyor; bu yüzden bağlantıyı düzenleyen düzeltmeler kapalı.
# Review: PO
hv-conn-fix-save-failed = Değişiklik bağlantılar dosyanıza kaydedilemedi.
# Review: PO
hv-conn-prod-title = { -brand-hacivat } { $name } için ne kadar dikkatli olsun?
# Review: PO
hv-conn-prod-body = { $host } bu Mac'te değil. Nasıl bir veritabanı olduğunu seçin. Bunu daha sonra bağlantı ayarlarından değiştirebilirsiniz.
# Review: PO
hv-conn-prod-suggest = Adreste “{ $token }” geçtiği için önerildi.
# Review: PO
hv-conn-prod-suggest-name = Bağlantı adında “{ $token }” geçtiği için önerildi.
# Review: PO
hv-conn-prod-opt-production = Üretim
# Review: PO
hv-conn-prod-opt-production-d = İnsanların ya da servislerin dayandığı canlı veri
# Review: PO
hv-conn-prod-opt-staging = Hazırlık
# Review: PO
hv-conn-prod-opt-staging-d = Üretimi yansıtan bir kopya
# Review: PO
hv-conn-prod-opt-test = Test
# Review: PO
hv-conn-prod-opt-test-d = Ortak test ya da CI verisi
# Review: PO
hv-conn-prod-opt-dev = Geliştirme
# Review: PO
hv-conn-prod-opt-dev-d = Bozulmasında sakınca olmayan geliştirme veritabanı
# Review: PO
hv-conn-prod-choices = Ortam
# Review: PO
hv-conn-prod-changes = Üretim seçilirse { -brand-hacivat }:
# Review: PO
hv-conn-prod-c1 = bu bağlantıda çalışırken başlık çubuğunda { $word } kapsülünü gösterir,
# Review: PO
hv-conn-prod-c1-word = PROD
# Review: PO
hv-conn-prod-c2 = veriyi ya da şemayı değiştiren ifadeler için onay ister,
# Review: PO
hv-conn-prod-c3 = elle commit kullanır: siz commit edene kadar hiçbir şey yazılmaz.
# Review: PO
hv-conn-prod-other = Ek onay yok. İfadeler çalıştıkça commit edilir.
# Review: PO
hv-conn-prod-save = { $env ->
        [production] Üretim olarak kaydet
        [staging] Hazırlık olarak kaydet
        [test] Test olarak kaydet
       *[dev] Geliştirme olarak kaydet
    }
# Review: PO
hv-conn-prod-pick = Bir ortam seçin
# Review: PO
hv-conn-prod-cancel = Vazgeç
# Review: PO
hv-conn-pw-title = { $name } oturumu
# Review: PO
hv-conn-pw-body = { $user } için parolayı girin. Yalnızca bağlantı kurulursa saklanır.
# Review: PO
hv-conn-pw-cancel = Vazgeç
## m2a-t17-dialog

# Review: PO
hv-cmd-connection-dialog-test = Bağlantıyı test et
# Review: PO
hv-cmd-connection-dialog-save = Bağlantıyı kaydet
# Review: PO
hv-cmd-connection-dialog-save-and-connect = Kaydet ve bağlan
# Review: PO
hv-cmd-connection-dialog-copy-url = Bağlantıyı URL olarak kopyala
# Review: PO
hv-conn-dlg-title-new = Yeni bağlantı
# Review: PO
hv-conn-dlg-title-edit = { $name } bağlantısını düzenle
# Review: PO
hv-conn-dlg-sub-new = Henüz kaydedilmedi
# Review: PO
hv-conn-dlg-sub-edit = Kayıtlı · değişiklikler bir sonraki bağlanışta geçerli olur
# Review: PO
hv-conn-dlg-url = URL
# Review: PO
hv-conn-dlg-url-ph = Formu doldurmak için bir bağlantı URL'si yapıştırın
# Review: PO
hv-conn-dlg-copy-url = URL olarak kopyala
# Review: PO
hv-conn-dlg-engine = Motor
# Review: PO
hv-conn-dlg-name = Ad
# Review: PO
hv-conn-dlg-env = Ortam
# Review: PO
hv-conn-dlg-env-ask = Bu adres bu Mac'te olmadığı için kaydederken sorulur.
# Review: PO
hv-conn-dlg-env-local = Yerel olarak ayarlandı: sunucu bu Mac'te çalışıyor.
# Review: PO
hv-conn-dlg-ro = Salt okunur
# Review: PO
hv-conn-dlg-ro-hint = · Veriyi ya da şemayı değiştiren ifadeleri engelle
# Review: PO
hv-conn-section-server = SUNUCU
# Review: PO
hv-conn-section-auth = KİMLİK DOĞRULAMA
# Review: PO
hv-conn-section-tls = Şifreleme (TLS)
# Review: PO
hv-conn-section-advanced = Gelişmiş
# Review: PO
hv-conn-dlg-host = Adres
# Review: PO
hv-conn-dlg-port = Port
# Review: PO
hv-conn-dlg-db = Veritabanı
# Review: PO
hv-conn-dlg-user = Kullanıcı
# Review: PO
hv-conn-dlg-file = Dosya
# Review: PO
hv-conn-dlg-choose = Seç…
# Review: PO
hv-conn-dlg-sqlite-ro = Dosyayı salt okunur aç
# Review: PO
hv-conn-dlg-sqlite-create = Dosya yoksa oluştur
# Review: PO
hv-conn-dlg-auth = Kimlik doğrulama
# Review: PO
hv-conn-auth-password = Parola
# Review: PO
hv-conn-auth-pgpass = pgpass
# Review: PO
hv-conn-auth-cert = İstemci sertifikası
# Review: PO
hv-conn-auth-none = Yok
# Review: PO
hv-conn-dlg-password = Parola
# Review: PO
hv-conn-dlg-remember = Hatırla
# Review: PO
hv-conn-dlg-pw-saved = Anahtar Zinciri'nde kayıtlı
# Review: PO
hv-conn-dlg-pw-from-url = yapıştırılan URL'den
# Review: PO
hv-conn-ret-never = Hiçbir zaman
# Review: PO
hv-conn-ret-never-h = her bağlanışta sor
# Review: PO
hv-conn-ret-restart = Yeniden başlatana kadar
# Review: PO
hv-conn-ret-restart-h = bellekte tutulur
# Review: PO
hv-conn-ret-session = Bu oturum boyunca
# Review: PO
hv-conn-ret-session-h = bağlantı kesilince unutulur
# Review: PO
hv-conn-ret-forever = Kalıcı (Anahtar Zinciri)
# Review: PO
hv-conn-ret-forever-h = macOS Anahtar Zinciri'nde saklanır
# Review: PO
hv-conn-ret-forever-off-h = sistemin anahtar deposu kullanılamıyor
# Review: PO
hv-conn-pgpass-match = ~/.pgpass dosyasının { $n }. satırıyla eşleşiyor
# Review: PO
hv-conn-pgpass-none = ~/.pgpass dosyasında bu bağlantıyla eşleşen bir satır henüz yok.
# Review: PO
hv-conn-pgpass-note = Her bağlanışta okunur. { -brand-hacivat } bu bağlantı için parola saklamaz.
# Review: PO
hv-conn-dlg-cert = Sertifika
# Review: PO
hv-conn-dlg-key = Özel anahtar
# Review: PO
hv-conn-auth-none-note = Parolasız bağlanır. Yalnızca bu makineye güvenen sunucularda kullanın.
# Review: PO
hv-conn-tls-sum-auto = Otomatik: sunucu TLS sunarsa doğrula
# Review: PO
hv-conn-tls-mode = Mod
# Review: PO
hv-conn-tls-auto = Otomatik
# Review: PO
hv-conn-tls-auto-h = sunulursa doğrula; şifresiz yalnızca özel ağlarda
# Review: PO
hv-conn-tls-auto-loopback-h = sunulursa doğrula; şifresiz yalnızca bu Mac'e
# Review: PO
hv-conn-tls-require = Zorunlu
# Review: PO
hv-conn-tls-require-h = şifrele, doğrulama
# Review: PO
hv-conn-tls-verify-ca = CA'yı doğrula
# Review: PO
hv-conn-tls-verify-ca-h = zinciri denetle
# Review: PO
hv-conn-tls-verify-full = Tam doğrula
# Review: PO
hv-conn-tls-verify-full-h = zinciri ve sunucu adını denetle
# Review: PO
hv-conn-tls-disable = Kapalı
# Review: PO
hv-conn-tls-disable-h = hiç şifreleme
# Review: PO
hv-conn-tls-disable-warn = Şifreleme kapalıyken parola ağda düz metin olarak taşınır; ağ yolundaki herkes onu okuyabilir.
# Review: PO
hv-conn-tls-ca = CA sertifikası
# Review: PO
hv-conn-tls-ca-ph = Sistem güven deposu
# Review: PO
hv-conn-adv-sum = Zaman aşımı { $timeout } sn · { $keepalive ->
        [on] { $seconds } sn'de bir canlı tutma
       *[off] canlı tutma kapalı
    }
# Review: PO
hv-conn-adv-timeout = Bağlanma zaman aşımı
# Review: PO
hv-conn-adv-timeout-unit = saniye
# Review: PO
hv-conn-adv-keepalive = Canlı tutma aralığı
# Review: PO
hv-conn-adv-off = Kapalı
# Review: PO
hv-conn-adv-every = { $seconds } sn
# Review: PO
hv-conn-adv-app = Uygulama adı
# Review: PO
hv-conn-adv-extra = Ek özellikler
# Review: PO
hv-conn-adv-extra-add = Özellik ekle
# Review: PO
hv-conn-adv-extra-key = anahtar
# Review: PO
hv-conn-adv-extra-value = değer
# Review: PO
hv-conn-adv-extra-remove = Özelliği kaldır
# Review: PO
hv-conn-adv-budget = Bağlantı sayısı
# Review: PO
hv-conn-adv-budget-v = Aynı anda en çok { $max } açık; fazlası sıra bekler
# Review: PO
hv-conn-dlg-test = Bağlantıyı test et
# Review: PO
hv-conn-dlg-test-stop = Durdur
# Review: PO
hv-conn-dlg-cancel = İptal
# Review: PO
hv-conn-dlg-save = Kaydet
# Review: PO
hv-conn-dlg-save-connect = Kaydet ve bağlan
# Review: PO
hv-conn-dlg-store-read-only = Bağlantılar dosyasına yazılamadığı için bağlantılar şu an kaydedilemiyor.
# Review: PO
hv-conn-dlg-save-failed = Bağlantı kaydedilemedi. Yeniden deneyin.
# Review: PO
hv-conn-dlg-save-gone = Bu bağlantı silindiği için değişiklikler kaydedilmedi.
# Review: PO
hv-conn-dlg-save-secret = Bağlantı kaydedildi, ancak parolası saklanamadı.
# Review: PO
hv-conn-env-local = yerel
# Review: PO
hv-conn-env-dev = geliştirme
# Review: PO
hv-conn-env-test = test
# Review: PO
hv-conn-env-staging = hazırlık
# Review: PO
hv-conn-env-production = üretim
# Review: PO
hv-conn-url-filled = Yapıştırılan URL'den dolduruldu.
# Review: PO
hv-conn-url-secret = Parola URL'den ayrıldı. Kaydettiğinizde Anahtar Zinciri'ne gider.
# Review: PO
hv-conn-url-secret-other = { $ret ->
        [never] Parola URL'den ayrıldı. { -brand-hacivat } onu saklamaz ve her bağlanışta sorar.
        [until_restart] Parola URL'den ayrıldı. { -brand-hacivat } onu yeniden başlatana kadar bellekte tutar.
       *[for_session] Parola URL'den ayrıldı. { -brand-hacivat } onu bu oturum boyunca tutar.
    }
# Review: PO
hv-conn-url-mapped = şuna dönüştü: { $setting }
# Review: PO
hv-conn-url-setting-tls = Şifreleme: { $mode }
# Review: PO
hv-conn-url-extra = ek özellik olarak saklandı
# Review: PO
hv-conn-url-engine-set = Motor URL'ye göre { $engine } olarak ayarlandı.
# Review: PO
hv-conn-url-undo = Geri al
# Review: PO
hv-conn-url-switch = Bu bir { $engine } URL'si, form ise { $current } için ayarlı.
# Review: PO
hv-conn-url-switch-l2 = Henüz hiçbir şey değişmedi.
# Review: PO
hv-conn-url-switch-btn = { $engine } motoruna geç ve doldur
# Review: PO
hv-conn-url-keep = { $current } kalsın
# Review: PO
hv-conn-url-unrec = { -brand-hacivat } bunu bir bağlantı URL'si olarak okuyamıyor. Hiçbir şey değişmedi.
# Review: PO
hv-conn-url-formats = postgres://, mysql://, mariadb://, jdbc:… URL'lerini, anahtar=değer dizelerini ve SQLite dosya yollarını okur.
# Review: PO
hv-conn-url-multihost = URL'de { $count } sunucu var. { -brand-hacivat } ilkine, { $host } adresine bağlanır.
# Review: PO
hv-conn-url-dismiss = Kapat
# Review: PO
hv-conn-toast-saved = { $name } kaydedildi
# Review: PO
hv-conn-toast-copied = URL olarak kopyalandı
# Review: PO
hv-conn-toast-nopw = Parola dahil değil.
# Review: PO
hv-conn-issue-name-missing = Bir ad girin.
# Review: PO
hv-conn-issue-host-missing = Bir adres girin.
# Review: PO
hv-conn-issue-host-invalid = Bu adres kullanılamıyor. Boşlukları ve URL karakterlerini kaldırın.
# Review: PO
hv-conn-issue-port-invalid = 1 ile 65535 arasında bir port girin.
# Review: PO
hv-conn-issue-file-missing = Bir veritabanı dosyası seçin ya da { -brand-hacivat } onu oluştursun.
# Review: PO
hv-conn-issue-path-refused = { -brand-hacivat } bu yolu açmaz. Yerel bir dosya seçin.
# Review: PO
hv-conn-issue-create-and-read-only = Yeni bir dosya salt okunur açılamaz. İkisinden birini kapatın.
# Review: PO
hv-conn-issue-transport-unavailable = Bu bağlantı, bu sürümün henüz açamadığı bir tünel kullanıyor.
# Review: PO
hv-conn-issue-tls-ca-unreadable = CA sertifikası okunamıyor. CA doğrulaması için bir sertifika gerekir.
# Review: PO
hv-conn-issue-tls-cert-unreadable = İstemci sertifikası ya da anahtarı okunamıyor.
# Review: PO
hv-conn-issue-tls-mode-unsupported = { $mode } bu motor için henüz kullanılamıyor.
# Review: PO
hv-conn-issue-option-rejected = { $key } özelliğine burada izin verilmiyor.

## m2a-t21-shots

# Review: PO
hv-conn-env-unset = ortam yok
# Review: PO
hv-conn-import-choose = Seçin…
# Review: PO
hv-conn-toast-delete-secret = Bağlantı silindi, ancak kayıtlı parolası sistemin anahtar deposundan kaldırılamadı.

## m2a-finalfix

# Review: PO
hv-conn-enc-ca-only = { $version } · CA doğrulandı
# Review: PO
hv-conn-issue-option-secret = { $key } özelliği bir parola taşıyor. Özelliği kaldırın, parolayı Parola alanına yazın.

## m2b-t01-interfaces

# Review: PO
hv-conn-issue-ssh-host-missing = SSH sunucusunu girin.
# Review: PO
hv-conn-issue-ssh-user-missing = Oturum açılacak kullanıcıyı girin.
# Review: PO
hv-conn-issue-ssh-too-many-hops = Bir tünel en fazla 8 ara sunucudan geçebilir.
# Review: PO
hv-conn-issue-ssh-key-file-missing = Bu anahtar dosyası bulunamıyor. ~/.ssh/id_ed25519 gibi özel anahtarı seçin.
# Review: PO
hv-conn-issue-tls-key-legacy-encrypted = Bu anahtar, { -brand-hacivat } uygulamasının okuyamadığı eski bir biçimde şifrelenmiş. Anahtarı şifreli bir PKCS #8 anahtarına dönüştürün (openssl pkcs8 -topk8), sonra yeni dosyayı seçin.

## m2b-t20-ssh

# Review: PO
hv-conn-section-ssh = SSH tüneli
# Review: PO
hv-conn-ssh-sum-off = Kapalı
# Review: PO
hv-conn-ssh-sum-on = { $host } üzerinden
# Review: PO
hv-conn-ssh-sum-hops = { $host } üzerinden · { $n } ara sunucu
# Review: PO
hv-conn-ssh-use = SSH tüneli üzerinden bağlan
# Review: PO
hv-conn-ssh-import = ~/.ssh/config'den içe aktar…
# Review: PO
hv-conn-ssh-host = SSH sunucusu
# Review: PO
hv-conn-ssh-user = SSH kullanıcısı
# Review: PO
hv-conn-ssh-auth-agent = Ajan
# Review: PO
hv-conn-ssh-auth-key = Anahtar dosyası
# Review: PO
hv-conn-ssh-auth-password = Parola
# Review: PO
hv-conn-ssh-auth-kbd = Klavye etkileşimli
# Review: PO
hv-conn-ssh-agent-note = ssh-agent'a yüklü anahtarları kullanır.
# Review: PO
hv-conn-ssh-key = Anahtar dosyası
# Review: PO
hv-conn-ssh-passphrase = Parola ifadesi
# Review: PO
hv-conn-ssh-kbd-note = Sunucunun soruları, örneğin tek kullanımlık kod, bağlanırken gösterilir.
# Review: PO
hv-conn-ssh-jumps = Ara sunucular
# Review: PO
hv-conn-ssh-add-jump = Ara sunucu ekle
# Review: PO
hv-conn-ssh-remove-jump = { $n }. ara sunucuyu kaldır
# Review: PO
hv-conn-ssh-jump-host-missing = Ara sunucuyu girin.
# Review: PO
hv-conn-ssh-jump-key-missing = Bir ara sunucunun anahtar dosyası bulunamıyor. ~/.ssh/config içindeki IdentityFile satırını düzeltip sunucuyu yeniden içe aktarın.
# Review: PO
hv-conn-ssh-route = Rota
# Review: PO
hv-conn-ssh-this-mac = { $os ->
        [mac] bu Mac
       *[other] bu bilgisayar
    }
# Review: PO
hv-conn-ssh-hostkeys = Sunucu anahtarları
# Review: PO
hv-conn-ssh-hk-ask = Yeni sunucularda sor
# Review: PO
hv-conn-ssh-hk-accept = Yeniyi kabul et, değişirse dur
# Review: PO
hv-conn-ssh-hk-strict = Yalnızca bilinen sunucular
# Review: PO
hv-conn-ssh-import-title = ~/.ssh/config içindeki sunucular
# Review: PO
hv-conn-ssh-proxycmd = ProxyCommand kullanıyor, desteklenmiyor
# Review: PO
hv-conn-ssh-unusable-secret = parolaya benzeyen bir değer içeriyor, içe aktarılmadı
# Review: PO
hv-conn-ssh-unusable-token = { $token } kullanıyor, desteklenmiyor
# Review: PO
hv-conn-ssh-unusable-depth = ara sunucuları çok derin iç içe
# Review: PO
hv-conn-ssh-unusable-unknown = dosyada olmayan bir ara sunucuyu adlandırıyor
# Review: PO
hv-conn-ssh-import-via = { $target } · ProxyJump { $jumps }
# Review: PO
hv-conn-ssh-import-key = { $target } · IdentityFile { $key }
# Review: PO
hv-conn-ssh-import-none = ~/.ssh/config içinde sunucu yok
# Review: PO
hv-conn-ssh-import-unreadable = ~/.ssh/config okunamıyor
# Review: PO
hv-conn-ssh-imported = { $alias }, ~/.ssh/config'den içe aktarıldı: adres, kullanıcı, anahtar ve ara sunucu.
# Review: PO
hv-conn-ssh-notice-match = { $line }. satır: Match blokları atlandı.
# Review: PO
hv-conn-ssh-notice-include = { $line }. satır: Include çok derin iç içe, adlandırdığı dosyalar atlandı.
# Review: PO
hv-conn-ssh-notice-known-hosts = { $alias }: UserKnownHostsFile yok sayıldı. { -brand-hacivat } kendi bilinen sunucular listesini ve ~/.ssh/known_hosts dosyasını denetler.
# Review: PO
hv-conn-ssh-notice-strict-no = { $alias }: StrictHostKeyChecking no, Yeni sunucularda sor olarak okundu.
# Review: PO
hv-conn-ssh-notice-agent = { $alias }: IdentityAgent yok sayıldı. { -brand-hacivat } SSH_AUTH_SOCK içindeki ajanı kullanır.
# Review: PO
hv-conn-ssh-notice-jump-password = { $alias }: ProxyJump bir parola içeriyordu; parola kaldırıldı ve saklanmadı.
# Review: PO
hv-conn-step-ssh = SSH

## m2a-demo-extra

# Review: PO
hv-conn-url-left-out = { $reason ->
        [unknown] { $engine } için bir bağlantı seçeneği değil, bu yüzden alınmadı
       *[refused] izin verilmiyor, bu yüzden alınmadı
    }
# Review: PO
hv-conn-err-property = Özellik
# Review: PO
hv-conn-err-path = Yol
# Review: PO
hv-conn-err-problem = Sorun
# Review: PO
hv-conn-err-refused = { $name } — { $reason ->
        [unknown] içe aktarılan bağlantılarda izin verilmiyor
        [encoding] iki kez kodlanmış
        [script] komut çalıştırıyor, içe aktarılan bağlantılarda izin verilmiyor
        [secret] bir parola taşıyor; parolayı Parola alanına yazın
        [path] { -brand-hacivat } bu yolu açmaz
       *[denied] hiçbir zaman izin verilmiyor
    }

## m2a-dialog-retries

# Review: PO
hv-conn-err-attempts-used = 3 denemenin hepsi kullanıldı. Hesap kilitlenmesin diye { -brand-hacivat } başka parola göndermez.
# Review: PO
hv-conn-dlg-attempts-used = Bu sunucu için deneme hakkı kalmadı. Yeniden denemek için adresi, portu ya da kullanıcıyı değiştirin veya bu pencereyi kapatıp yeniden açın.

## m2b-t22-live

# Review: PO
hv-capsule-reconnecting = Yeniden bağlanıyor · { $n }. deneme
# Review: PO
hv-capsule-next = { $seconds } sn sonra
# Review: PO
hv-conn-path-ssh = { $host } üzerinden SSH
# Review: PO
hv-conn-idle-cap = Bağlantı kesik (boşta)
# Review: PO
hv-conn-idle-disconnected = { $name } bağlantısı { $hours } sa boşta kaldıktan sonra kesildi. Kullandığınızda yeniden bağlanır.
# Review: PO
hv-conn-idle-disconnected-minutes = { $name } bağlantısı { $minutes } dk boşta kaldıktan sonra kesildi. Kullandığınızda yeniden bağlanır.
# Review: PO
hv-conn-idle-tx = { $name } üzerindeki bir işlem { $minutes } dakikadır açık ve boşta. Commit edilene ya da geri alınana kadar kilitlerini tutar.
# Review: PO
hv-conn-lost-title = { $state ->
        [reconnected] { $name } ile yeniden bağlanıldı
       *[lost] { $name } oturumu kayboldu
    }
# Review: PO
hv-conn-lost-body = { $state ->
        [reconnected] Sunucu oturumu sona erdi; { -brand-hacivat } yeni bir oturum açtı.
       *[lost] Sunucu oturumu süren bir iş varken sona erdi; bu yüzden { -brand-hacivat } kendiliğinden yeniden bağlanmadı. Yeniden dene ile tekrar bağlanabilirsiniz.
    }
# Review: PO
hv-conn-lost-tx = Açık işleminiz sona erdi ve sunucu commit edilmemiş değişikliklerini geri aldı.
# Review: PO
hv-conn-lost-other = Ayrıca kayboldu: { $items }
# Review: PO
hv-conn-lost-item-temp = geçici tablolar
# Review: PO
hv-conn-lost-item-vars = oturum değişkenleri
# Review: PO
hv-conn-lost-item-prepared = { $count } hazırlanmış ifade
# Review: PO
hv-conn-lost-item-locks = oturum kilitleri
# Review: PO
hv-conn-lost-details = Ayrıntıları göster
# Review: PO
hv-conn-lost-dismiss = Kapat
# Review: PO
hv-conn-gap-title = { $name } üzerinde salt okunur koruma
# Review: PO
hv-conn-gap-tx-ended = { -brand-hacivat }, salt okunur bir bağlantıda salt okunur işlemi sonlandıran bir ifadeyi durdurdu ve o oturumu kapattı.
# Review: PO
hv-conn-gap-changed-data = { -brand-hacivat }, salt okunur bir bağlantıda veri değiştiren bir ifadeyi durdurdu ve o oturumu kapattı.
# Review: PO
hv-conn-access-ro-enforced = salt okunur (sunucu rolüyle zorunlu)
# Review: PO
hv-conn-ro-sheet-enforced = salt okunur · zorunlu
# Review: PO
hv-conn-ro-sheet-guarded = salt okunur · korumalı
# Review: PO
hv-conn-ro-pop-enf-title = Salt okunur, sunucu tarafından zorunlu
# Review: PO
hv-conn-ro-pop-enf-body = { $user } rolü burada yalnızca okuyabilir. { -brand-hacivat } içinde ne çalışırsa çalışsın, yazmaları sunucunun kendisi reddeder.
# Review: PO
hv-conn-ro-pop-enf-temp = Tek başına TEMP yetkisi zorunluluğu zayıflatmaz; etiket zorunlu kalır.
# Review: PO
hv-conn-ro-pop-g-title = Salt okunurluk korumalı, zorunlu değil
# Review: PO
hv-conn-ro-pop-g-body = { $user } rolü burada yalnızca okumayla sınırlı değil; bu yüzden yazmaları { -brand-hacivat } kendisi durdurur: her ifadeyi denetler ve salt okunur çalıştırır.
# Review: PO
hv-conn-ro-pop-g-limit = Bu, hataları yakalar. Kesin güvence ancak sunucudaki salt okunur bir rolle sağlanır.
# Review: PO
hv-conn-ro-pop-g-fix = Zorunlu hale getirmek için veritabanı yöneticinizden böyle bir rol isteyin ve onu bu bağlantının kullanıcısı yapın:
# Review: PO
hv-conn-ro-pop-maria-body = { -brand-hacivat } bir MariaDB rolünün salt okunur olduğunu henüz doğrulayamıyor; bu yüzden MariaDB bağlantılarını her zaman kendisi korur.
# Review: PO
hv-conn-ro-pop-no-fix = Hiçbir rol değişikliği bu etiketi zorunlu yapmaz; bu yüzden kopyalanacak SQL yok.
# Review: PO
hv-conn-ro-pop-sqlite-body = SQLite'ta sunucu rolü yoktur. { -brand-hacivat } dosyayı salt okunur açar ve yalnızca okuyan ifadelere izin verir.
# Review: PO
hv-conn-ro-copy-sql = SQL'i kopyala
# Review: PO
hv-conn-ro-copied = SQL kopyalandı. Çalıştırmadan önce köşeli parantez içindeki değerleri değiştirin.
# Review: PO
hv-conn-ro-warn-head = Bu rolde bulunanlar:
# Review: PO
hv-conn-ro-warn-superuser = Süper kullanıcı.
# Review: PO
hv-conn-ro-warn-bypass-rls = Satır düzeyi güvenliği atlıyor.
# Review: PO
hv-conn-ro-warn-replication = Çoğaltma başlatabilir.
# Review: PO
hv-conn-ro-warn-create-role = Rol oluşturup değiştirebilir.
# Review: PO
hv-conn-ro-warn-server-file-roles = Veritabanı sunucusundaki dosyaları okuyabilir ya da yazabilir.
# Review: PO
hv-conn-ro-warn-execute-server-program = Veritabanı sunucusunda program çalıştırabilir.
# Review: PO
hv-conn-ro-warn-signal-backend = Başka oturumları iptal edebilir ya da sonlandırabilir.
# Review: PO
hv-conn-ro-warn-write-privileges = Bazı tablolardaki verileri değiştirebilir.
# Review: PO
hv-conn-ro-warn-temp-privilege = Geçici tablo oluşturabilir; salt okunur işlemler bunları engeller.
# Review: PO
hv-conn-ro-warn-egress-extension = { $name } eklentisi başka sunuculara erişebilir.
# Review: PO
hv-conn-ro-warn-role-grant-not-expanded = Ona başka roller verilmiş; { -brand-hacivat } bunların neye izin verdiğini göremiyor.
# Review: PO
hv-conn-ro-warn-unparsed = { -brand-hacivat } yetkilerini okuyamadı.
# Review: PO
hv-cmd-connection-explain-access = Salt okunur erişimi açıkla

## m2b-t21-prompts

# Review: PO
hv-conn-hk-title = { $host } ile ilk bağlantı
# Review: PO
hv-conn-hk-body = { $os ->
        [mac] Bu Mac bu SSH sunucusuna daha önce bağlanmadı. Parmak izini yöneticinizin verdiğiyle karşılaştırın.
       *[other] Bu bilgisayar bu SSH sunucusuna daha önce bağlanmadı. Parmak izini yöneticinizin verdiğiyle karşılaştırın.
    }
# Review: PO
hv-conn-hk-fp = Sunucu anahtarı
# Review: PO
hv-conn-hk-store = { -brand-hacivat } bunu kendi bilinen sunucular listesine kaydeder. ~/.ssh/known_hosts değişmez.
# Review: PO
hv-conn-hk-trust = Güven ve bağlan
# Review: PO
hv-conn-hk-copy-fp = Parmak izini kopyala

# Review: PO
hv-conn-hk-changed-title = { $host } sunucusunun anahtarı değişti
# Review: PO
hv-conn-hk-changed-body = Sunucu, kayıtlı anahtarından farklı bir anahtarla yanıt verdi. Bu, sunucu yeniden kurulduğunda ya da anahtarları yenilendiğinde olur; biri bağlantının arasına girdiğinde de olur. { -brand-hacivat } oturum açmadan önce durdu.
# Review: PO
hv-conn-hk-changed-old = Kayıtlı anahtar
# Review: PO
hv-conn-hk-changed-new = Şimdi gönderilen
# Review: PO
hv-conn-hk-changed-stored-in = Kayıtlı olduğu yer
# Review: PO
hv-conn-hk-changed-src-hacivat = Hacivat'ın bilinen sunucuları
# Review: PO
hv-conn-hk-changed-src-openssh = { $file }, { NUMBER($n, useGrouping: "false") }. satır
# Review: PO
hv-conn-hk-changed-ask = { $host } sunucusunu yöneten kişiye anahtarın değişip değişmediğini sorun ve güvenmeden önce yeni parmak izini karşılaştırın.
# Review: PO
hv-conn-hk-changed-replace = Eski anahtarı sil, yenisine güven
# Review: PO
hv-conn-hk-changed-openssh-note = { -brand-hacivat } { $file } dosyasını değiştirmez. Yeni anahtar gerçekse eski satırı bir terminalde silin, sonra yeniden deneyin:
# Review: PO
hv-conn-hk-changed-copy = Komutu kopyala
# Review: PO
hv-conn-hk-changed-copied = Komut kopyalandı
# Review: PO
hv-conn-hk-changed-trusted = { $host } sunucusunun yeni anahtarına güvenildi

# Review: PO
hv-conn-kbd-title = { $host } oturumu
# Review: PO
hv-conn-kbd-asks = { $host } soruyor:
# Review: PO
hv-conn-kbd-from-server = Sunucunun gönderdiği metin, olduğu gibi
# Review: PO
hv-conn-kbd-no-text = Sunucu bir ad ya da açıklama göndermedi.
# Review: PO
hv-conn-kbd-attempt = 3 denemeden { $n }.
# Review: PO
hv-conn-kbd-not-kept = Yanıtlar yalnızca bu sunucuya gider ve saklanmaz.
# Review: PO
hv-conn-kbd-rejected = Sunucu yanıtları kabul etmedi ve yeniden sordu.
# Review: PO
hv-conn-kbd-continue = Devam et

# Review: PO
hv-conn-ssh-secret-hop = { $host }, { $total } atlamadan { $n }.
# Review: PO
hv-conn-ssh-secret-pw-title = { $host } için SSH parolası
# Review: PO
hv-conn-ssh-secret-pw-body = { $user } için SSH parolasını girin. Yalnızca bağlantı kurulursa saklanır.
# Review: PO
hv-conn-ssh-secret-pass-title = { $key } için parola ifadesi
# Review: PO
hv-conn-ssh-secret-pass-body = Bu anahtar dosyası şifreli. Parola ifadesini girin. Yalnızca bağlantı kurulursa saklanır.
# Review: PO
hv-conn-ssh-secret-wrong = { $host } parolayı kabul etmedi.
# Review: PO
hv-conn-ssh-secret-pass-wrong = Bu parola ifadesi anahtarı açmadı.
# Review: PO
hv-conn-ssh-secret-attempt = 3 denemeden { $n }.
# Review: PO
hv-conn-ssh-secret-continue = Devam et

# Review: PO
hv-conn-vault-create-title = Kayıtlı parolalar için bir kasa oluşturun
# Review: PO
hv-conn-vault-create-why = Bu bilgisayarda sistem anahtarlığı yok.
# Review: PO
hv-conn-vault-create-what = { -brand-hacivat }, kalıcı olarak kaydettiğiniz parolaları ana parolayla kilitlenen şifreli bir dosyada tutar.
# Review: PO
hv-conn-vault-master = Ana parola
# Review: PO
hv-conn-vault-repeat = Tekrar
# Review: PO
hv-conn-vault-min = En az 8 karakter
# Review: PO
hv-conn-vault-match = İki giriş eşleşiyor
# Review: PO
hv-conn-vault-mismatch = İki giriş eşleşmiyor
# Review: PO
hv-conn-vault-norecover = { -brand-hacivat } bu parolayı kurtaramaz. Unutursanız kasayı sıfırlar ve kayıtlı parolalarınızı yeniden girersiniz.
# Review: PO
hv-conn-vault-notnow-note = Şimdi değil derseniz bu parola { -brand-hacivat } yeniden başlayana kadar tutulur.
# Review: PO
hv-conn-vault-notnow = Şimdi değil
# Review: PO
hv-conn-vault-create-btn = Kasayı oluştur
# Review: PO
hv-conn-vault-created = Kasa oluşturuldu. Kalıcı kaydettiğiniz parolalar oraya gider.
# Review: PO
hv-conn-vault-unlock-title = Kasanın kilidini açın
# Review: PO
hv-conn-vault-unlock-body = Hacivat'ın { $name } için kayıtlı parolayı okuyabilmesi için ana parola gerekiyor. Kasa, uygulamadan çıkana kadar açık kalır.
# Review: PO
hv-conn-vault-unlock-body-any = Hacivat'ın kayıtlı parolalarınızı okuyabilmesi için ana parola gerekiyor. Kasa, uygulamadan çıkana kadar açık kalır.
# Review: PO
hv-conn-vault-unlock-wrong = Bu, ana parola değil.
# Review: PO
hv-conn-vault-attempt = 3 denemeden { $n }.
# Review: PO
hv-conn-vault-unlock-btn = Kilidi aç
# Review: PO
hv-conn-vault-reset-link = Kasayı sıfırla…
# Review: PO
hv-conn-vault-reset-title = Kasa sıfırlansın mı?
# Review: PO
hv-conn-vault-reset-body = Bu işlem kasayı ve içindeki bütün kayıtlı parolaları siler. Bağlantılarınız kalır; { -brand-hacivat } her parolayı bir sonraki bağlanışta sorar. Bu işlem geri alınamaz.
# Review: PO
hv-conn-vault-reset-type = Onaylamak için { $word } yazın
# Review: PO
hv-conn-vault-reset-word = reset
# Review: PO
hv-conn-vault-reset-btn = Kasayı sıfırla
# Review: PO
hv-conn-vault-reset-done = Kasa sıfırlandı. İçindeki kayıtlı parolalar silindi.
# Review: PO
hv-conn-vault-damaged-title = Kasa okunamıyor
# Review: PO
hv-conn-vault-damaged-body = Kasa dosyası bütünlük denetiminden geçemedi; { -brand-hacivat } dosyayı kullanmadı ve değiştirmedi. Bir yedekten geri yükleyin ya da kasayı sıfırlayın.
# Review: PO
hv-conn-vault-damaged-meanwhile = O zamana kadar girdiğiniz parolalar { -brand-hacivat } yeniden başlayana kadar tutulur.
# Review: PO
hv-conn-ret-forever-vault = Kalıcı (şifreli dosya)
# Review: PO
hv-conn-ret-forever-vault-h = Hacivat'ın kasasında, ana parolayla

# Review: PO
hv-conn-fix-review-host-key = Sunucu anahtarını incele…

# Review: PO
hv-conn-ssh-secret-remember-all = Bu bağlantının tüm atlamalarındaki kayıtlı SSH parolalarına ve parola ifadelerine uygulanır.
# Review: PO
hv-conn-save-unchanged = Hiçbir şey değiştirilmedi: kayıtlı parolalar taşınamadı ya da silinemedi. { -brand-hacivat } sorarsa kasanın kilidini açın, sonra yeniden deneyin.
# Review: PO
hv-conn-vault-denied = { -brand-hacivat } kasa dosyasını kullanamıyor: dosya normal bir dosya değil. Girdiğiniz parolalar { -brand-hacivat } yeniden başlayana kadar tutulur.

## m2b-t19-tls

# Review: PO
hv-conn-fix-review-cert = Sertifikayı incele…
# Review: PO
hv-conn-fix-import-ca = CA içe aktar…
# Review: PO
hv-conn-tls-title = Sertifikaya güvenilmiyor
# Review: PO
hv-conn-tls-body = { $host }:{ $port }, bu Mac'in güvendiği hiçbir CA'nın imzalamadığı bir sertifika sundu. Devam etmeden önce parmak izini sunucuyu yöneten kişiyle karşılaştırın.
# Review: PO
hv-conn-tls-subject = Verildiği ad
# Review: PO
hv-conn-tls-issuer = Veren
# Review: PO
hv-conn-tls-valid = Geçerlilik sonu
hv-conn-tls-valid-date = { DATETIME($when, dateStyle: "long") }
# Review: PO
hv-conn-tls-fp = SHA-256 parmak izi
# Review: PO
hv-conn-tls-copy-fp = Parmak izini kopyala
# Review: PO
hv-conn-tls-chain = Zincir
# Review: PO
hv-conn-tls-leaf = sunucu
# Review: PO
hv-conn-tls-inter = ara sertifika
# Review: PO
hv-conn-tls-root = kök · bu Mac güvenmiyor
# Review: PO
hv-conn-tls-pin-note = Sabitleme, { $host }:{ $port } için yalnızca bu sertifikaya güvenir. Sertifika bir gün değişirse { -brand-hacivat } durur ve yeniden sorar. Sabitleme geçerlilik süresini uzatmaz.
# Review: PO
hv-conn-tls-pin-btn = Bu sertifikayı sabitle
# Review: PO
hv-conn-tls-import = CA içe aktar…
# Review: PO
hv-conn-tls-dates-note = { $state ->
        [expired] Bu sertifikanın süresi dolmuş. Sabitleme geçerlilik süresini uzatmadığı için sertifika sabitlenemez. Sunucuyu yöneten kişiden sertifikayı yenilemesini isteyin.
       *[early] Bu sertifika henüz geçerli değil. Bu Mac'in tarih ve saatini denetleyin ya da sunucuyu yöneten kişiye sorun.
    }
# Review: PO
# Fix round 1 (review M2): shorter than the mock's "Sabitlenmiş sertifika", which the 140 px
# form label column cut off; it reads with "Sabitlemeyi kaldır" beside it.
hv-conn-tls-pin = Sabitleme
# Review: PO
hv-conn-tls-pin-none = Yok
# Review: PO
hv-conn-tls-remove-pin = Sabitlemeyi kaldır
# Review: PO
hv-conn-issue-tls-disable-ignores-pin = Şifreleme kapalıyken sabitlenmiş sertifika denetlenmez. Sabitlemeyi kullanmak için başka bir mod seçin.
# Review: PO
hv-conn-dlg-key-pass = Anahtar parolası
# Review: PO
hv-conn-issue-tls-key-passphrase = Bu anahtar şifreli. Parolasını girin.
# Review: PO
hv-conn-tls-pin-changed-title = { $host }:{ $port } sertifikası değişti
# Review: PO
hv-conn-tls-pin-changed-body = Bu bağlantı tek bir sertifikaya sabitlendi; sunucu şimdi farklı bir sertifika sunuyor. { -brand-hacivat } oturum açmadan önce durdu.
# Review: PO
hv-conn-tls-pin-changed-old = Sabitlenen
# Review: PO
hv-conn-tls-pin-changed-new = Şimdiki
# Review: PO
hv-conn-tls-pin-changed-ask = Yenilenmiş bir sertifika da böyle görünür. Sabitlemeyi değiştirmeden önce sunucuyu yöneten kişiye sorun.
# Review: PO
hv-conn-tls-pin-changed-note = Sabitlemeyi değiştirmek, { $host }:{ $port } için yalnızca yeni sertifikaya güvenir ve eski sabitlemeyi kaldırır. Sabitleme geçerlilik süresini uzatmaz.
# Review: PO
hv-conn-tls-pin-changed-replace = Sabitlemeyi değiştir
# Review: PO
hv-conn-tls-pin-changed-done = { $host } sunucusunun yeni sertifikası sabitlendi
# Review: PO
hv-conn-plain-title = { $host } şifreleme sunmuyor
# Review: PO
hv-conn-plain-body = { $ip } yalnızca şifresiz bağlantı kabul ediyor ve özel bir ağda değil. Parolanız ve sorgu sonuçlarınız ağda düz metin olarak taşınır.
# Review: PO
hv-conn-plain-ssh = SSH tüneli kullan…
# Review: PO
hv-conn-plain-connect = Şifrelemeden bağlan
# Review: PO
hv-conn-plain-remember = Bu bağlantı için bir daha sorma

## m2b-t23-f1

# Review: PO
hv-conn-ro-warn-non-transactional-write = Etkileri ROLLBACK ile geri alınamayan fonksiyonlar çalıştırabilir.

## m2b-t24-shots

# Review: PO
hv-conn-enc-pinned = { $version } · sabitlendi
# Review: PO
hv-conn-diag-tls-pinned = TLS { $version } · sabitlendi
# Review: PO
hv-conn-diag-tls-nov-pinned = TLS · sabitlendi
# Review: PO
hv-conn-fix-ssh = SSH üzerinden bağlan…
# Review: PO
hv-conn-secret-vault = kasa · kalıcı

## m3a-t01b-seams

# Review: PO
hv-menu-edit = Düzen

## m3a-t25-editor-commands

# Review: PO
hv-cmd-editor-move-left = Sola git
# Review: PO
hv-cmd-editor-move-right = Sağa git
# Review: PO
hv-cmd-editor-move-up = Yukarı git
# Review: PO
hv-cmd-editor-move-down = Aşağı git
# Review: PO
hv-cmd-editor-select-left = Seçimi sola genişlet
# Review: PO
hv-cmd-editor-select-right = Seçimi sağa genişlet
# Review: PO
hv-cmd-editor-select-up = Seçimi yukarı genişlet
# Review: PO
hv-cmd-editor-select-down = Seçimi aşağı genişlet
# Review: PO
hv-cmd-editor-move-word-left = Önceki sözcüğe git
# Review: PO
hv-cmd-editor-move-word-right = Sonraki sözcüğe git
# Review: PO
hv-cmd-editor-select-word-left = Seçimi önceki sözcüğe genişlet
# Review: PO
hv-cmd-editor-select-word-right = Seçimi sonraki sözcüğe genişlet
# Review: PO
hv-cmd-editor-move-line-start = Satır başına git
# Review: PO
hv-cmd-editor-move-line-end = Satır sonuna git
# Review: PO
hv-cmd-editor-select-line-start = Seçimi satır başına genişlet
# Review: PO
hv-cmd-editor-select-line-end = Seçimi satır sonuna genişlet
# Review: PO
hv-cmd-editor-move-document-start = Belge başına git
# Review: PO
hv-cmd-editor-move-document-end = Belge sonuna git
# Review: PO
hv-cmd-editor-select-document-start = Seçimi belge başına genişlet
# Review: PO
hv-cmd-editor-select-document-end = Seçimi belge sonuna genişlet
# Review: PO
hv-cmd-editor-page-up = Sayfa yukarı
# Review: PO
hv-cmd-editor-page-down = Sayfa aşağı
# Review: PO
hv-cmd-editor-select-page-up = Seçimi bir sayfa yukarı genişlet
# Review: PO
hv-cmd-editor-select-page-down = Seçimi bir sayfa aşağı genişlet
# Review: PO
hv-cmd-editor-newline = Satır sonu ekle
# Review: PO
hv-cmd-editor-tab = Girintile
# Review: PO
hv-cmd-editor-outdent = Girintiyi azalt
# Review: PO
hv-cmd-editor-delete-backward = Geriye doğru sil
# Review: PO
hv-cmd-editor-delete-forward = İleriye doğru sil
# Review: PO
hv-cmd-editor-delete-word-backward = Önceki sözcüğü sil
# Review: PO
hv-cmd-editor-delete-word-forward = Sonraki sözcüğü sil
# Review: PO
hv-cmd-editor-delete-to-line-start = Satır başına kadar sil
# Review: PO
hv-cmd-editor-select-all = Tümünü seç
# Review: PO
hv-cmd-editor-undo = Geri al
# Review: PO
hv-cmd-editor-redo = Yinele
# Review: PO
hv-cmd-editor-copy = Kopyala
# Review: PO
hv-cmd-editor-cut = Kes
# Review: PO
hv-cmd-editor-paste = Yapıştır
# Review: PO
hv-cmd-editor-escape = Seçimi daralt
# Review: PO
hv-cmd-editor-toggle-tab-moves-focus = Sekme tuşuyla odak taşımayı aç veya kapat
# Review: PO
hv-cmd-editor-context-menu = Düzenleyici menüsünü göster
# Review: PO
hv-cmd-editor-run = Çalıştır
# Review: PO
hv-cmd-editor-run-2 = Çalıştır (2)
# Review: PO
hv-cmd-editor-run-3 = Çalıştır (3)
# Review: PO
hv-cmd-editor-describe-exec-range = Çalıştırma aralığını açıkla
# Review: PO
hv-editor-describe-one = Çalıştır, { $from }–{ $to }. satırları çalıştırır: { $label }
# Review: PO
hv-editor-describe-one-line = Çalıştır, { $line }. satırı çalıştırır: { $label }
# Review: PO
hv-editor-describe-many = Çalıştır, { $from }–{ $to }. satırlardaki { $n } ifadeyi çalıştırır
# Review: PO
hv-editor-describe-many-line = Çalıştır, { $line }. satırdaki { $n } ifadeyi çalıştırır
# Review: PO
hv-editor-describe-ask = Çalıştır, hangi ifadenin çalıştırılacağını sorar
# Review: PO
hv-editor-describe-nothing = Burada çalıştırılacak bir şey yok
# Review: PO
hv-editor-describe-uncertain = Sınır belirsiz: çalıştırmak için ifadeyi seçin
# Review: PO
hv-editor-describe-offline = Bağlı değil: Çalıştır önce bağlanır ve hiçbir şey çalıştırmaz
# Review: PO
hv-editor-describe-pending = Düzenleyici bu metni hâlâ okuyor. Birazdan yeniden deneyin.
# Review: PO
hv-editor-tab-moves-focus-on = Sekme tuşu odağı taşır
# Review: PO
hv-editor-tab-moves-focus-off = Sekme tuşu girinti ekler

## m3a-t26-results-model

# Review: PO
hv-results-result-n = Sonuç { $n }
# Review: PO
hv-grid-truncated = … { $size }
# Review: PO
hv-grid-size-bytes = { $n } B
# Review: PO
hv-grid-size-kb = { $n } KB
# Review: PO
hv-grid-size-mb = { $n } MB
# Review: PO
hv-grid-size-gb = { $n } GB
# Review: PO
hv-grid-size-chars = { $n } karakter
# Review: PO
hv-grid-decode-error = Çözümlenemedi
# Review: PO
hv-copy-done = { $rows } × { $cols } hücre kopyalandı
# Review: PO
hv-copy-done-line = Sekmeyle ayrılmış, ham değerler
# Review: PO
hv-copy-done-incomplete = Kopyalanan değerlerden { $n } tanesi eksik: kısaltılmış ya da çözümlenememiş.
# Review: PO
hv-copy-working = { $cells } hücre kopyalanıyor…
# Review: PO
hv-copy-refused-title = Kopyalanamayacak kadar büyük
# Review: PO
hv-copy-refused-body = Seçimde { $cells } hücre var. Kopyalama en çok { $max } hücre ya da { $size } alır; daha az satır ya da sütun seçin.
# Review: PO
hv-copy-refused-multiple = Bu seçimler birlikte kopyalanamaz. Tek bir blok ya da aynı satırları veya sütunları paylaşan bloklar seçin.

## m3a-t34-status-follows

# Review: PO
hv-tx-open = İşlem açık · { $n } ifade
# Review: PO
hv-tx-open-tip = BEGIN'den bu yana çalışan ifadeler henüz commit edilmedi. Komut paletinden commit edin ya da geri alın.
# Review: PO
hv-tx-failed = İşlem başarısız · bitirmek için geri alın
# Review: PO
hv-tx-failed-tip = Bir hata ya da iptal işlemi durdurdu. Sunucu artık yalnızca geri almayı kabul eder.
# Review: PO
hv-results-rows = { $rows } satır
# Review: PO
hv-results-rows-more = { $rows }+ satır
# Review: PO
hv-results-rows-timing = { $rows } satır · { $duration }
# Review: PO
hv-results-rows-more-timing = { $rows }+ satır · { $duration }
# Review: PO
hv-status-duration-ms = { $ms } ms
# Review: PO
hv-status-duration-s = { NUMBER($seconds, minimumFractionDigits: 2) } sn
# Review: PO
hv-results-status-cursor = İmleç açık
# Review: PO
hv-results-status-cursor-tip = Veritabanı anlık görüntüsü tutuluyor. İmleci kapatmak için tıklayın.

## m3a-t28-gutter

# Review: PO
hv-editor-nothing-to-run = Burada çalıştırılacak bir şey yok
# Review: PO
hv-editor-uncertain = Sınır belirsiz: çalıştırmak için ifadeyi seçin
# Review: PO
hv-editor-preview-unit = { $i }/{ $n }
# Review: PO
hv-editor-preview-offline = bağlı değil
# Review: PO
hv-editor-preview-uncertain = sınır belirsiz
# Review: PO
hv-editor-mark-stale = çalıştırıldıktan sonra düzenlendi
# Review: PO
hv-editor-tip-stale = Çalıştırıldıktan sonra düzenlendi. İşaretler son çalıştırmayı gösterir.
# Review: PO
hv-editor-tip-two = Bu satırda iki ifade var

## m3a-t32-console

# Review: PO
hv-console-tab-title = { $name } ({ $n })
# Review: PO
hv-console-run = Çalıştır
# Review: PO
hv-console-stop = Durdur
# Review: PO
hv-console-running = Çalışıyor
# Review: PO
hv-console-clock = { $s } sn
# Review: PO
hv-console-cancelling = İptal ediliyor…
# Review: PO
hv-console-waiting = { $name } için boş bağlantı bekleniyor (kullanımda: { $inUse }/{ $max })
# Review: PO
hv-console-waiting-short = Bağlantı bekleniyor (kullanımda: { $inUse }/{ $max })
# Review: PO
hv-console-waiting-cancel = Vazgeç
# Review: PO
hv-editor-lncol = Sat { NUMBER($line, useGrouping: "false") }, Süt { NUMBER($col, useGrouping: "false") }
# Review: PO
hv-editor-selected = { $n } seçili
# Review: PO
hv-editor-reduced = Kısıtlı mod
# Review: PO
hv-console-size-mb = { NUMBER($mb, minimumFractionDigits: 1, maximumFractionDigits: 1) } MB
# Review: PO
hv-console-restore-refused-title = { $name } yeniden açılamadı
# Review: PO
hv-console-restore-refused-body = Kayıtlı metni { $size }; bu sürüm en çok { $limit } açabiliyor. Dosya değiştirilmeden { $file } adıyla saklandı.
# Review: PO
hv-console-restore-unreadable-body = Kayıtlı metni okunamadı. Dosya değiştirilmeden { $file } adıyla saklandı.
# Review: PO
hv-edit-undo-typing = Yazmayı Geri Al
# Review: PO
hv-edit-undo-paste = Yapıştırmayı Geri Al
# Review: PO
hv-edit-undo-drop = Bırakmayı Geri Al
# Review: PO
hv-edit-undo-delete = Silmeyi Geri Al
# Review: PO
hv-edit-undo-cut = Kesmeyi Geri Al
# Review: PO
hv-edit-undo-indent = Girintiyi Geri Al
# Review: PO
hv-edit-undo-outdent = Girinti Azaltmayı Geri Al
# Review: PO
hv-edit-redo-typing = Yazmayı Yinele
# Review: PO
hv-edit-redo-paste = Yapıştırmayı Yinele
# Review: PO
hv-edit-redo-drop = Bırakmayı Yinele
# Review: PO
hv-edit-redo-delete = Silmeyi Yinele
# Review: PO
hv-edit-redo-cut = Kesmeyi Yinele
# Review: PO
hv-edit-redo-indent = Girintiyi Yinele
# Review: PO
hv-edit-redo-outdent = Girinti Azaltmayı Yinele
# Review: PO
hv-console-ro-pending = { $name }, açık işlemi commit edildiğinde ya da geri alındığında salt okunur olur.
# Review: PO
hv-console-ro-pending-short = Bu işlemden sonra salt okunur
# Review: PO
hv-console-session-failed = { $name } için bağlantı açılamadı. Çalıştır yeniden dener.
# Review: PO
hv-console-release-busy = Önce açık soruyu yanıtlayın, sonra yeniden deneyin.
## m3a-t30-results-view

# Review: PO
hv-results-output = Çıktı
# Review: PO
hv-results-unread = Yeni çıktı
# Review: PO
hv-results-tabs = Sonuç sekmeleri
# Review: PO
hv-results-env-suffix = ({ $env })
# Review: PO
hv-results-copy = Kopyala
# Review: PO
hv-results-log-clear = Temizle
# Review: PO
hv-results-log-end = Sona kaydır
# Review: PO
hv-results-log-empty = Henüz çıktı yok. Çalıştırdığınız ifadelerin iletileri burada görünür.
# Review: PO
hv-results-log-label = Çıktı günlüğü
# Review: PO
hv-grid-no-rows = Satır yok
# Review: PO
hv-grid-too-many-columns = { $n } sütun. Geniş sonuçlar yana kayar; daha rahat okumak için daha az sütun seçin.
# Review: PO
hv-grid-loading = Yükleniyor
# Review: PO
hv-strip-fetching = Getiriliyor… { $rows } satır
# Review: PO
hv-strip-rows-more = { $rows }+ satır
# Review: PO
hv-strip-cursor-open = İmleç açık — veritabanı anlık görüntüsü tutuluyor
# Review: PO
hv-strip-cursor-open-short = İmleç açık
# Review: PO
hv-strip-close-cursor = İmleci kapat
# Review: PO
hv-strip-fetch-more = Daha fazla getir
# Review: PO
hv-strip-fetch-all = Tümünü getir
# Review: PO
hv-strip-complete = { $rows } satır
# Review: PO
hv-strip-rerun = Yeniden çalıştır
# Review: PO
hv-strip-cancelled = { $rows } satırdan sonra iptal edildi
# Review: PO
hv-strip-memory = Bu sonucun bellek sınırına ulaşıldı ({ $rows } satır, { $size })
# Review: PO
hv-strip-fetch-anyway = Yine de getir
# Review: PO
hv-strip-truncated = { $rows } satır tutuldu: bu sonucun bellek sınırına ulaşıldı
# Review: PO
hv-strip-closed-idle = { $rows } satırdan sonra imleç kapandı: çok uzun süre boşta kaldı.
# Review: PO
hv-strip-more-sets = { $n } sonuç kümesi daha gösterilmedi
# Review: PO
hv-strip-selection = Seçim
# Review: PO
hv-strip-dims = { $rows } × { $cols }
# Review: PO
hv-cmd-results-copy = Seçili sonuç hücrelerini kopyala
# Review: PO
hv-cmd-results-fetch-more = Sonucun daha fazla satırını getir
# Review: PO
hv-cmd-results-fetch-all = Sonucun tüm satırlarını getir
# Review: PO
hv-cmd-results-close-cursor = Sonucun imlecini kapat
# Review: PO
hv-cmd-results-rerun = Sonucun ifadesini yeniden çalıştır
# Review: PO
hv-cmd-grid-move-left = Tablo: sola git
# Review: PO
hv-cmd-grid-move-right = Tablo: sağa git
# Review: PO
hv-cmd-grid-move-up = Tablo: yukarı git
# Review: PO
hv-cmd-grid-move-down = Tablo: aşağı git
# Review: PO
hv-cmd-grid-select-left = Tablo: seçimi sola genişlet
# Review: PO
hv-cmd-grid-select-right = Tablo: seçimi sağa genişlet
# Review: PO
hv-cmd-grid-select-up = Tablo: seçimi yukarı genişlet
# Review: PO
hv-cmd-grid-select-down = Tablo: seçimi aşağı genişlet
# Review: PO
hv-cmd-grid-jump-left = Tablo: ilk sütuna atla
# Review: PO
hv-cmd-grid-jump-right = Tablo: son sütuna atla
# Review: PO
hv-cmd-grid-jump-up = Tablo: ilk satıra atla
# Review: PO
hv-cmd-grid-jump-down = Tablo: yüklenen son satıra atla
# Review: PO
hv-cmd-grid-home = Tablo: satırın ilk hücresine git
# Review: PO
hv-cmd-grid-end = Tablo: satırın son hücresine git
# Review: PO
hv-cmd-grid-first-row = Tablo: ilk satıra git
# Review: PO
hv-cmd-grid-last-row = Tablo: son satıra git
# Review: PO
hv-cmd-grid-page-up = Tablo: sayfa yukarı
# Review: PO
hv-cmd-grid-page-down = Tablo: sayfa aşağı
# Review: PO
hv-cmd-grid-next-cell = Tablo: sonraki hücre
# Review: PO
hv-cmd-grid-prev-cell = Tablo: önceki hücre
# Review: PO
hv-cmd-grid-select-all = Tablo: tümünü seç
# Review: PO
hv-cmd-grid-select-row = Tablo: satırı seç
# Review: PO
hv-cmd-grid-select-column = Tablo: sütunu seç
# Review: PO
hv-cmd-grid-copy = Tablo: kopyala
# Review: PO
hv-cmd-grid-copy-with-headers = Tablo: başlıklarla kopyala
# Review: PO
hv-cmd-grid-escape = Tablo: seçimi temizle
## m3a-t33-exec-ui

# Review: PO
hv-cmd-exec-cancel = Çalışan ifadeyi durdur
# Review: PO
hv-cmd-tx-commit = İşlemi commit et
# Review: PO
hv-cmd-tx-rollback = İşlemi geri al
# Review: PO
hv-console-connect-first = { $name } bağlantısı kuruluyor. Bağlandıktan sonra yeniden çalıştırın.
# Review: PO
hv-editor-mark-queued = sırada
# Review: PO
hv-editor-mark-running = çalışıyor · { $duration }
# Review: PO
hv-editor-mark-rows = çalıştı · { $duration } · { $rows } satır
# Review: PO
hv-editor-mark-rows-more = çalıştı · { $duration } · { $rows }+ satır
# Review: PO
hv-editor-mark-affected = { $rows } satır etkilendi · { $duration }
# Review: PO
hv-editor-mark-done = çalıştı · { $duration }
# Review: PO
hv-editor-mark-failed = başarısız
# Review: PO
hv-editor-mark-denied = gönderilmedi: salt okunur
# Review: PO
hv-editor-mark-cancelling = iptal ediliyor…
# Review: PO
hv-editor-mark-cancelled = { $duration } sonra iptal edildi
# Review: PO
hv-editor-mark-skipped = atlandı: önceki bir ifade başarısız oldu
# Review: PO
hv-editor-mark-short-done = { $duration }
# Review: PO
hv-editor-mark-short-running = { $duration }
# Review: PO
hv-editor-preview-choices = { $n } seçenek
# Review: PO
hv-chooser-title = Neyin çalışacağını seçin
# Review: PO
hv-chooser-statement = İfade
# Review: PO
hv-chooser-subquery = Alt sorgu
# Review: PO
hv-chooser-largest = En geniş ifade
# Review: PO
hv-chooser-from-caret = İmleçten sona
# Review: PO
hv-chooser-whole = Tüm betik
# Review: PO
hv-chooser-row = { $n ->
        [one] { $kind }
       *[other] { $kind } · { $n } ifade
    }
# Review: PO
hv-chooser-uncertain-title = Sınır belirsiz
# Review: PO
hv-chooser-uncertain-body = Hacivat bu ifadenin tam olarak nerede bittiğini anlayamıyor; bu yüzden hiçbir şey göndermedi.
# Review: PO
hv-chooser-uncertain-fix = Çalıştırılacak metni tam olarak seçin, sonra { $key } tuşuna basın.
# Review: PO
hv-chooser-uncertain-fix-unbound = Çalıştırılacak metni tam olarak seçin, sonra çalıştırın.
# Review: PO
hv-tx-close-title = { $name } üzerinde { $n } ifadelik açık bir işlem var
# Review: PO
hv-tx-close-body = Bu konsolu kapatmak işlemini sonlandırır. Commit değişiklikleri korur, geri alma onları atar.
# Review: PO
hv-tx-restart-body = Yeniden başlatmak oturumu sonlandırır. Commit değişiklikleri korur, geri alma onları atar.
# Review: PO
hv-tx-commit-restart = Commit et ve yeniden başlat
# Review: PO
hv-tx-rollback-restart = Geri al ve yeniden başlat
# Review: PO
hv-tx-release-body = Konsolun bağlantısı değişmek üzere; bu, işlemi sonlandırır. Commit değişiklikleri korur, geri alma onları atar.
# Review: PO
hv-tx-commit-continue = Commit et ve devam et
# Review: PO
hv-tx-rollback-continue = Geri al ve devam et
# Review: PO
hv-tx-quit-body = Çıkmak oturumu sonlandırır. Commit değişiklikleri korur, geri alma onları atar.
# Review: PO
hv-tx-commit-close = Commit et ve kapat
# Review: PO
hv-tx-rollback-close = Geri al ve kapat
# Review: PO
hv-tx-commit-quit = Commit et ve çık
# Review: PO
hv-tx-rollback-quit = Geri al ve çık
# Review: PO
hv-tx-cancel = Vazgeç
# Review: PO
hv-tx-failed-title = { $name } üzerindeki işlem başarısız oldu
# Review: PO
hv-tx-failed-body = Bir hata ya da iptal onu durdurdu; artık yalnızca geri alınabilir. İçindeki { $n } ifade korunmayacak.
# Review: PO
hv-tx-end-failed = { $name } üzerindeki işlem sona ermedi; hiçbir şey kapatılmadı.
# Review: PO
hv-tx-failed-by-stop = İfadeyi durdurmak { $name } üzerindeki işlemi başarısız kıldı. Artık yalnızca geri alma onu sonlandırabilir.
# Review: PO
hv-exec-pending-read-only = Hiçbir şey gönderilmedi: bu konsol, açık işlemi sona erdiğinde salt okunur olur. Önce commit edin ya da geri alın.
# Review: PO
hv-exec-rerun-edited = İfade çalıştırıldıktan sonra düzenlendi. Düzenleyiciden çalıştırın.

## m3b-t01b-seams

# Review: PO
hv-cmd-editor-add-caret-above = Üste imleç ekle
# Review: PO
hv-cmd-editor-add-caret-below = Alta imleç ekle
# Review: PO
hv-cmd-editor-select-next-occurrence = Sonraki eşleşmeyi seç
# Review: PO
hv-cmd-editor-select-all-occurrences = Tüm eşleşmeleri seç
# Review: PO
hv-cmd-editor-split-selection-into-lines = Seçimi satırlara böl
# Review: PO
hv-cmd-editor-undo-selection = Seçim değişikliğini geri al
# Review: PO
hv-cmd-editor-redo-selection = Seçim değişikliğini yinele
# Review: PO
hv-cmd-editor-fold = Katla
# Review: PO
hv-cmd-editor-unfold = Katlamayı aç
# Review: PO
hv-cmd-editor-fold-all = Tümünü katla
# Review: PO
hv-cmd-editor-unfold-all = Tüm katlamaları aç
# Review: PO
hv-cmd-editor-find = Bul
# Review: PO
hv-cmd-editor-find-next = Sonrakini bul
# Review: PO
hv-cmd-editor-find-prev = Öncekini bul
# Review: PO
hv-cmd-editor-find-close = Bulmayı kapat
# Review: PO
hv-cmd-editor-toggle-comment = Yorum satırı yap veya kaldır
# Review: PO
hv-cmd-editor-duplicate-line = Satırı çoğalt
# Review: PO
hv-cmd-editor-delete-line = Satırı sil
# Review: PO
hv-cmd-editor-move-line-up = Satırı yukarı taşı
# Review: PO
hv-cmd-editor-move-line-down = Satırı aşağı taşı
# Review: PO
hv-cmd-editor-normalize-line-endings = Satır sonlarını eşitle
# Review: PO
hv-cmd-editor-history-prev = Geçmişten önceki ifade
# Review: PO
hv-cmd-editor-history-next = Geçmişten sonraki ifade
# Review: PO
hv-cmd-grid-grow-selection = Tablo: seçimi genişlet
# Review: PO
hv-cmd-grid-shrink-selection = Tablo: seçimi daralt
# Review: PO
hv-cmd-grid-find = Tablo: bul
# Review: PO
hv-cmd-grid-goto-row = Tablo: satıra git
# Review: PO
hv-cmd-grid-open-value = Tablo: değer görüntüleyiciyi aç
# Review: PO
hv-cmd-grid-context-menu = Tablo: menüyü göster
# Review: PO
hv-cmd-grid-auto-fit = Tablo: sütun genişliklerini sığdır
# Review: PO
hv-cmd-grid-freeze = Tablo: buraya kadar sütunları dondur
# Review: PO
hv-cmd-grid-unfreeze = Tablo: sütunları çöz
# Review: PO
hv-cmd-grid-sort-asc = Tablo: artan sırala
# Review: PO
hv-cmd-grid-sort-desc = Tablo: azalan sırala
# Review: PO
hv-cmd-grid-clear-sort = Tablo: sıralamayı kaldır
# Review: PO
hv-cmd-grid-view-table = Tablo: tablo görünümü
# Review: PO
hv-cmd-grid-view-transposed = Tablo: devrik görünüm
# Review: PO
hv-cmd-grid-view-text = Tablo: metin görünümü
# Review: PO
hv-cmd-grid-toggle-record = Tablo: kayıt görünümünü göster veya gizle
# Review: PO
hv-cmd-results-next-tab = Sonraki sonuç sekmesi
# Review: PO
hv-cmd-results-prev-tab = Önceki sonuç sekmesi
# Review: PO
hv-cmd-history-open = Sorgu geçmişini aç

## m3a-f4-ime
# Review: PO
hv-cmd-app-character-palette = Emoji ve Semboller

## m3b-t07-types

# Review: PO
hv-results-type-mysql-code = MySQL türü { $code }

## m3b-t22-tx-ui

# Review: PO
hv-tx-chip-auto = Otomatik commit
# Review: PO
hv-tx-chip-manual = Manuel
# Review: PO
hv-tx-chip-smart = Akıllı
# Review: PO
hv-tx-chip-tip = İşlem kipi: { $mode }
# Review: PO
hv-tx-menu-title = İşlem kipi
# Review: PO
hv-tx-mode-auto-hint = her ifade çalışınca commit edilir
# Review: PO
hv-tx-mode-manual-hint = değişiklikler Commit'i bekler
# Review: PO
hv-tx-mode-smart-hint = ilk değişiklikten itibaren Manuel
# Review: PO
hv-tx-menu-isolation = Yalıtım düzeyi
# Review: PO
hv-tx-iso-default = Sunucu varsayılanı
# Review: PO
hv-tx-iso-default-hint = Sunucu varsayılanı · { $level }
# Review: PO
hv-tx-menu-reason-open = Önce commit edin ya da geri alın
# Review: PO
hv-tx-menu-prod-note = Üretim konsolları Manuel kipte başlar.
# Review: PO
hv-tx-chip-ro-reason = Salt okunur bir oturum her ifadeyi tek başına çalıştırır; bu yüzden kipi her zaman Otomatik commit'tir.
# Review: PO
hv-tx-badge = İşlem · { $n } ifade · { $age }
# Review: PO
hv-tx-badge-short = İşlem · { $n } ifade
# Review: PO
hv-tx-badge-failed = İşlem başarısız
# Review: PO
hv-tx-badge-tip = Henüz commit edilmedi: diğer oturumlar bu değişiklikleri görmez ve dokunulan satırlar kilitli kalır.
# Review: PO
hv-tx-badge-failed-tip = Bir hata ya da iptal işlemi durdurdu. Sunucu artık yalnızca geri almayı kabul eder.
# Review: PO
hv-tx-badge-commit = Commit et
# Review: PO
hv-tx-badge-rollback = Geri al
# Review: PO
hv-tx-segment-tip = Commit, geri alma ve işlem kipi için tıklayın.
# Review: PO
hv-tx-idle-title = { $name } üzerinde { $age } süredir açık bir işlem var
# Review: PO
hv-tx-idle-body = Diğer oturumları engelleyen kilitler tutuyor olabilir.
# Review: PO
hv-tx-idle-commit = Commit et
# Review: PO
hv-tx-idle-rollback = Geri al
# Review: PO
hv-tx-idle-keep = Açık bırak
# Review: PO
hv-tx-lost-maybe = Açık bir işlemi olmuş olabilir; öyleyse sunucu commit edilmemiş değişikliklerini geri aldı.
# Review: PO
hv-conn-sessions = Oturumlar
# Review: PO
hv-conn-single-session = Tüm konsollar için tek oturum kullan
# Review: PO
hv-conn-single-session-sub = Bu bağlantının konsolları tek oturumu paylaşır: işlemini, değişkenlerini ve geçici tablolarını.
# Review: PO
hv-conn-budget-single = Tüm konsolların paylaştığı tek oturum
# Review: PO
hv-console-shared = Paylaşılan oturum
# Review: PO
hv-console-shared-tip = { $name } bağlantısının tüm konsolları bu oturumu kullanır: burada açılan bir işlem hepsinde açıktır.

## m3b-t22-tx-ui-r1

# Review: PO
hv-tx-idle-waits = Konsol, çalıştırmasıyla ilgili bir yanıt bekliyor. Yanıtlandıktan sonra commit edin ya da geri alın.
# Review: PO
hv-conn-single-session-reconnect = { $name } için oturum ayarı yeniden bağlandıktan sonra geçerli olur.
# Review: PO
hv-conn-zone-refused = Sunucu { $zone } saat dilimini reddetti

## m3b-t21-confirm

# Review: PO
hv-confirm-title = Bu ifade { $name } üzerinde çalıştırılsın mı?
# Review: PO
hv-confirm-title-warn = Bu ifade { $name } üzerinde satırları kilitler
# Review: PO
hv-confirm-sub-unit = ifade { $i }/{ $n }
# Review: PO
hv-confirm-sql = İfade
# Review: PO
hv-confirm-effect = Etkisi
# Review: PO
hv-confirm-prod-line = Siz seçene kadar hiçbir şey gönderilmez.
# Review: PO
hv-confirm-calm-line = Üretim dışında Hacivat yalnızca bunun gibi ifadelerden önce sorar. Ayarlar › Çalıştırma bölümünden kapatılabilir.
# Review: PO
hv-confirm-class-lock = Satırları okur ve kilitler
# Review: PO
hv-confirm-class-dml = Veriyi değiştirir
# Review: PO
hv-confirm-class-dml-unsafe = Veriyi kullanılabilir bir filtre olmadan değiştirir
# Review: PO
hv-confirm-class-ddl-destructive = Şema nesnelerini ya da verilerini kaldırır
# Review: PO
hv-confirm-class-ddl-other = Şemayı değiştirir
# Review: PO
hv-confirm-class-routine = Bir yordam çalıştırır
# Review: PO
hv-confirm-class-admin = Sunucuyu yönetir ya da veritabanının dışına erişir
# Review: PO
hv-confirm-class-tx = İşlemi denetler
# Review: PO
hv-confirm-class-unknown = Bilinmiyor: Hacivat ne yaptığını anlayamıyor
# Review: PO
hv-confirm-mysql-ddl = MySQL DDL'i hemen commit eder; bu yüzden bu işlem geri alınamaz.
# Review: PO
hv-confirm-type-label = Çalıştırmak için { $name } yazın
# Review: PO
hv-confirm-type-hint = Büyük-küçük harf dahil, tam gösterildiği gibi yazın.
# Review: PO
hv-confirm-copy-name = Adı kopyala
# Review: PO
hv-confirm-execute = Çalıştır
# Review: PO
hv-confirm-execute-all = Tümünü çalıştır
# Review: PO
hv-confirm-execute-suppress = Çalıştır, bir daha sorma
# Review: PO
hv-confirm-execute-suppress-prod = Çalıştır; bu konsolda bir daha sorma
# Review: PO
hv-confirm-suppress-scope = Aynı biçimdeki ifadeler { $console } kapanana kadar yeniden sorulmaz.
# Review: PO
hv-confirm-skip = Atla
# Review: PO
hv-confirm-skip-all = Tümünü atla
# Review: PO
hv-confirm-cancel = Vazgeç
# Review: PO
hv-confirm-execute-all-scope = Tümünü çalıştır, sorulacak kalan { $n } ifadeyi de sormadan çalıştırır. Adının yazılması gereken bir ifade yine sorar; üretimdeki bir değişiklik de çalıştıktan sonra yine yanıtınızı bekler.

# Review: PO
hv-commit-title = { $name } üzerinde { $rows } satır değişti
# Review: PO
hv-commit-title-unknown = İfade { $name } üzerinde veriyi değiştirdi
# Review: PO
hv-commit-body-own = Hacivat bu ifadeyi kendi açtığı bir işlemde çalıştırdı. Commit edene kadar diğer oturumlar değişikliği görmez.
# Review: PO
hv-commit-locks = Yanıt beklenirken satırlar kilitli kalır. Yanıt gelmezse Hacivat 5 dakika sonra geri alır.
# Review: PO
hv-commit-commit = Commit et
# Review: PO
hv-commit-rollback = Geri al
# Review: PO
hv-commit-countdown = { $time } sonra geri alınır
# Review: PO
hv-commit-sp-title = İşleminizin içinde { $rows } satır değişti
# Review: PO
hv-commit-sp-title-unknown = İfade, işleminizin içinde veriyi değiştirdi
# Review: PO
hv-commit-sp-keep-line = Koru: değişiklik açık işleminize katılır. Henüz hiçbir şey commit edilmez.
# Review: PO
hv-commit-sp-undo-line = İfadeyi geri al: yalnızca bu ifadenin değişikliği kaldırılır; işleminiz açık kalır.
# Review: PO
hv-commit-sp-locks = Yanıt gelmezse Hacivat 5 dakika sonra ifadeyi geri alır ve işleminizi açık bırakır.
# Review: PO
hv-commit-keep = Koru
# Review: PO
hv-commit-undo = İfadeyi geri al
# Review: PO
hv-commit-sp-countdown = İfade { $time } sonra geri alınır
# Review: PO
hv-commit-dropped-title = { $name } bağlantısı koptu
# Review: PO
hv-commit-dropped-body = Sunucu bu değişiklikleri geri aldı. Hiçbir şey commit edilmedi.
# Review: PO
hv-commit-dropped-sp-body = Sunucu açık işleminizi geri aldı: bu ifade ve ondan önceki { $n } ifade. Hiçbir şey commit edilmedi.
# Review: PO
hv-commit-dropped-sp-body-unknown = Sunucu, bu ifade de içinde olmak üzere açık işleminizi geri aldı. Hiçbir şey commit edilmedi.
# Review: PO
hv-commit-close = Kapat
# Review: PO
hv-confirm-type-untypeable = Bu ad yazılamayan ya da yapıştırılamayan bir karakter içeriyor; bu yüzden burada doğrulanamaz. Bu ifadeyi başka bir istemciden çalıştırın.

## m3b-t25-results

# Review: PO
hv-results-tab-dup = { $title } ({ $n })
# Review: PO
hv-results-tab-pinned = Sabitlendi: bir sonraki çalıştırma yeni sekme açar
# Review: PO
hv-results-tab-pin = Sekmeyi sabitle
# Review: PO
hv-results-tab-unpin = Sabitlemeyi kaldır
# Review: PO
hv-results-tab-close-others = Diğer sekmeleri kapat
# Review: PO
hv-results-untyped = türsüz (tek parça)
# Review: PO
hv-results-untyped-short = türsüz
# Review: PO
hv-results-untyped-tip = Tek parça çalıştırma ifadeleri birlikte gönderir; bu yüzden sunucu her sütunu metin olarak döndürür. Sayılar metin gibi sıralanır.
# Review: PO
hv-results-set-limit = İlk { $kept } sonuç kümesi gösteriliyor; { $skipped } küme daha okundu ve tutulmadı.
# Review: PO
hv-results-count = Satırları say
# Review: PO
hv-results-count-tip = Bu sorgu üzerinde ayrı bir oturumda SELECT COUNT(*) çalıştırır.
# Review: PO
hv-results-count-exact = { $rows } satır (kesin)
# Review: PO
hv-results-counting = Sayılıyor…
# Review: PO
hv-results-count-cancel = Vazgeç
# Review: PO
hv-results-count-unavailable = Bu sorgu sayılamaz.
# Review: PO
hv-results-count-refused = Sayılmadı: Hacivat sayımın yalnızca okuduğunu doğrulayamadı.
# Review: PO
hv-results-count-timeout = 30 saniye sonra durduruldu.
# Review: PO
hv-results-count-failed = Sayım başarısız oldu.
# Review: PO
hv-results-count-offline = Sayılmadı: konsol bağlı değil.
# Review: PO
hv-strip-evicted = Bellek açmak için kapatıldı; ilk { $rows } satır tutuldu.
# Review: PO
hv-results-copy-evicted = Hiçbir şey kopyalanmadı: bu sonuç bellek açmak için kapatıldı ve seçtiğinizden daha az satır tutuyor. Yeniden seçin ya da yeniden çalıştırın.
# Review: PO
hv-fetchall-title = Tüm satırları getirmek bu sonucun bellek sınırını aşar
# Review: PO
hv-fetchall-body = Sorgunun satırları { $size } sınırına sığmıyor. Hacivat sınıra kadar getirip kalanı için imleci açık tutabilir.
# Review: PO
hv-fetchall-ok = Bellek sınırına kadar getir
# Review: PO
hv-fetchall-cancel = Vazgeç

## m3b-t25-results-r1

# Review: PO
hv-results-tab-repeat = ({ $n })
# Review: PO
hv-results-tabs-more = +{ $n }
# Review: PO
hv-results-tabs-more-tip = { $n } sonuç sekmesi daha
# Review: PO
hv-results-count-rerun = Satırları saymak için ifadeyi yeniden çalıştırın.
# Review: PO
hv-results-count-context = Sayılmadı: konsolun SQL ayarları henüz bilinmiyor.

## m3b-t23-escalation

# Review: PO
hv-cancel-slow-title = İptal uzun sürüyor
# Review: PO
hv-cancel-slow-body = { $duration } geçti; sunucu { $name } üzerindeki ifadeyi hâlâ durdurmadı.
# Review: PO
hv-cancel-options = Seçenekleri göster…
# Review: PO
hv-cancel-dlg-title = İfade iptale yanıt vermiyor
# Review: PO
hv-cancel-dlg-body = { $duration } süredir iptal ediliyor. Siz başka bir şey seçmedikçe Hacivat beklemeyi sürdürür.
# Review: PO
hv-cancel-keep-waiting = Beklemeyi sürdür
# Review: PO
hv-cancel-terminate = Sunucuda sonlandır
# Review: PO
hv-cancel-terminate-line = “Sunucuda sonlandır” bu oturumu sunucuda { $fn } ile kapatır.
# Review: PO
hv-cancel-own-tx = Bu konsolun açık işlemi ({ $n } ifade) geri alınır.
# Review: PO
hv-cancel-own-tx-maybe = Bu konsolun açık bir işlemi olabilir; öyleyse geri alınır.
# Review: PO
hv-cancel-own-tx-hacivat = Hacivat'ın bu konsolda açtığı işlem (yanıtınızı bekleyen bir değişiklik ya da açık bir imleç) geri alınır.
# Review: PO
hv-cancel-other-tx-hacivat = { $console } konsolunda da Hacivat'ın açtığı bir işlem var; sunucu onu geri alır.
# Review: PO
hv-cancel-disconnect = Bağlantıyı kes
# Review: PO
hv-cancel-disconnect-line-all = “Bağlantıyı kes”, { $name } için buradaki tüm bağlantıları kapatır; ifade sunucuda çalışmayı sürdürebilir.
# Review: PO
hv-cancel-disconnect-line-one = “Bağlantıyı kes” yalnızca bu konsolun oturumunu ({ $console }) kapatır; ifade sunucuda çalışmayı sürdürebilir.
# Review: PO
hv-cancel-disconnect-line-local = “Bağlantıyı kes”, bu bağlantının konsollarının paylaştığı { $name } dosyası bağlantısını kapatır; SQLite, bağlantı kapanınca ifadeyi durdurur.
# Review: PO
hv-cancel-other-tx = { $console } konsolunda da açık bir işlem var ({ $n } ifade); sunucu onu geri alır.
# Review: PO
hv-cancel-other-tx-maybe = { $console } konsolunun da açık bir işlemi olabilir; öyleyse sunucu onu geri alır.
# Review: PO
hv-cancel-terminate-failed = Oturum sonlandırılamadı. Hacivat iptali beklemeyi sürdürüyor.
# Review: PO
hv-editor-mark-escalated = iptal yanıtlanmadı · { $duration }
# Review: PO
hv-errbar-failed = { $i }. ifade başarısız oldu ({ $n } ifadeden)
# Review: PO
hv-errbar-aborted = { $i }. ifade başarısız oldu ({ $n } ifadeden); işlem de başarısız oldu
# Review: PO
hv-errbar-waiting = { $n } ifade daha bekliyor
# Review: PO
hv-errbar-retry = Yeniden dene
# Review: PO
hv-errbar-ignore = Yok say
# Review: PO
hv-errbar-ignore-all = Tümünü yok say
# Review: PO
hv-errbar-stop = Durdur
# Review: PO
hv-errbar-rollback-continue = Geri al ve devam et
# Review: PO
hv-errbar-rollback-continue-tip = Açık işleminizi ({ $n } ifade) geri alır, sonra { $next }. ifadeden yeni bir işlemde devam eder.
# Review: PO
hv-errbar-rollback-continue-tip-auto = Açık işleminizi ({ $n } ifade) geri alır, sonra { $next }. ifadeden devam eder; her ifade çalışınca commit edilir.

## m3b-t24-exec-ui

# Review: PO
hv-params-title = Bu ifadenin değerleri
# Review: PO
hv-params-sub = { $n } parametre
# Review: PO
hv-params-col-name = Ad
# Review: PO
hv-params-col-value = Değer
# Review: PO
hv-params-col-type = Tür
# Review: PO
hv-params-null-tip = NULL gönder
# Review: PO
hv-params-remembered = hatırlandı
# Review: PO
hv-params-remembered-tip = Bu uygulama oturumundaki önceki bir çalıştırmadan; diske hiç kaydedilmez.
# Review: PO
hv-params-ambiguous = Ondalık { $value } olarak okunur: burada nokta ondalık ayırıcıdır. { $plain } demek istediyseniz noktasız yazın.
# Review: PO
hv-params-ambiguous-use = { $value } kullan
# Review: PO
hv-params-bad-hex = \x ile yazılan baytlar onaltılık rakam çiftleri (0–9, a–f) olmalıdır.
# Review: PO
hv-params-template = { $var } bir şablon değişkeni, parametre değil. Hacivat onu doldurmaz; bu yüzden ifade bu hâliyle çalıştırılamaz.
# Review: PO
hv-params-pg-q = PostgreSQL'de ? burada parametre değildir, çünkü jsonb işleçleri onu kullanır. Ayarlar › Parametreler bölümünden açılabilir.
# Review: PO
hv-params-note = Değerler yalnızca bu uygulama oturumu boyunca tutulur ve geçmişte maskelenir.
# Review: PO
hv-params-run = Çalıştır
# Review: PO
hv-params-cancel = Vazgeç
# Review: PO
hv-params-resolve-first = Önce işaretli değeri düzeltin
# Review: PO
hv-params-type-unknown = bilinmiyor
# Review: PO
hv-chooser-run-as-one = Tek parça çalıştır
# Review: PO
hv-chooser-run-as-one-hint = Tek parça çalıştırma bölgenin tamamını tek istekte gönderir; sonuçları türsüz gelir.
# Review: PO
hv-chooser-run-as-one-ro = Salt okunur oturumda kullanılamaz: koruma her seferinde tek bir ifadeyi denetler.
# Review: PO
hv-chooser-customize = Özelleştir…
# Review: PO
hv-policy-title = Bu konsolda Çalıştır hangi ifadeleri seçer
# Review: PO
hv-policy-inside = İmleç bir ifadenin içindeyken
# Review: PO
hv-policy-inside-ask = Sor
# Review: PO
hv-policy-inside-smallest = En küçük ifade
# Review: PO
hv-policy-inside-largest = En geniş ifade
# Review: PO
hv-policy-inside-whole = Tüm betik
# Review: PO
hv-policy-selection = Seçim varken
# Review: PO
hv-policy-sel-separate = Ayrı ifadeler olarak
# Review: PO
hv-policy-sel-as-one = Tek parça olarak
# Review: PO
hv-policy-note = Bu konsol kapanana kadar geçerli.
# Review: PO
hv-editor-mark-awaiting = Commit ya da geri alma bekleniyor
# Review: PO
hv-editor-mark-awaiting-sp = Yanıt bekleniyor: Koru ya da İfadeyi geri al
# Review: PO
hv-editor-mark-not-sent = gönderilmedi
# Review: PO
hv-exec-history-recovered = Sorgu geçmişi dosyası bozulmuştu; Hacivat onu bir kenara ayırıp yenisini başlattı.

## m3b-t27-history

# Review: PO
hv-history-title = Sorgu geçmişi
# Review: PO
hv-history-search = Geçmişte ara
# Review: PO
hv-history-search-tip = Arama büyük-küçük harfi ve aksanları yok sayar: musteri, müşteri'yi bulur.
# Review: PO
hv-history-filter-this = Bu bağlantı
# Review: PO
hv-history-filter-all = Tüm bağlantılar
# Review: PO
hv-history-outcome-any = Tüm sonuçlar
# Review: PO
hv-history-outcome-ok = Başarılı
# Review: PO
hv-history-outcome-failed = Başarısız
# Review: PO
hv-history-outcome-cancelled = İptal edildi
# Review: PO
hv-history-outcome-denied = Çalıştırılmadı
# Review: PO
hv-history-today = Bugün
# Review: PO
hv-history-yesterday = Dün
# Review: PO
hv-history-count = { $n } kayıt
# Review: PO
hv-history-rows = { $rows } satır
# Review: PO
hv-history-redacted = gizlendi
# Review: PO
hv-history-redacted-line = Kaydedilmeden önce bir parola ya da gizli değer çıkarıldı.
# Review: PO
hv-history-pin = Sabitle
# Review: PO
hv-history-unpin = Sabitlemeyi kaldır
# Review: PO
hv-history-pinned = Sabitlendi: saklama süresi dolsa da silinmez
# Review: PO
hv-history-insert = Ekle
# Review: PO
hv-history-insert-run = Ekle ve çalıştır
# Review: PO
hv-history-copy = SQL'i kopyala
# Review: PO
hv-history-open-new = Yeni konsolda aç
# Review: PO
hv-history-empty = Henüz geçmiş yok. Çalıştırdığınız ifadeler burada 90 gün tutulur.
# Review: PO
hv-history-nomatch = “{ $q }” ile eşleşen bir şey yok.
# Review: PO
hv-history-ran = { $n } kez çalıştı
# Review: PO
hv-history-target = { $name } konsoluna eklenir; Çalıştır denetimi orada yapılır.
hv-history-time = { DATETIME($t, timeStyle: "medium") }
hv-history-day = { DATETIME($t, dateStyle: "medium") }
# Review: PO
hv-history-conn-gone = Silinmiş bağlantı
# Review: PO
hv-history-failed = Geçmiş şu anda okunamıyor.
# Review: PO
hv-history-masked-line = Maskelenmiş değerler var: çalıştırmadan önce bunları doldurun.
# Review: PO
hv-history-truncated-line = Bu ifade kaydedilirken 64 KiB'de kesildi; olduğu gibi çalıştırılamaz.
# Review: PO
hv-history-count-capped = { $n }+ kayıt
# Review: PO
hv-history-console-gone = Eklenecek konsol kapalı.
# Review: PO
hv-history-insert-refused = İfade eklenemedi: düzenleyici salt okunur, metin oluşturuluyor ya da çok büyüyecek.
# Review: PO
hv-history-pin-failed = Sabitleme değiştirilemedi.
# Review: PO
hv-history-open-failed = Yeni konsol zamanında açılmadı; hiçbir şey eklenmedi.

## m3b-t31-multi

# Review: PO
hv-editor-carets = { $n } imleç
# Review: PO
hv-editor-ime-primary-only = Giriş yöntemi: başka bir imleç bir dizenin ya da yorumun içinde olduğu için yalnızca ana imleçte yazılıyor

## m3b-t29-shots
# Review: PO
hv-history-selection-a11y = { $sql }, { $n } kayıttan { $pos }.

## m3b-t32-fold

# Review: PO
hv-editor-fold-lines = { $n } satır
# Review: PO
hv-editor-fold-tip = { $from }–{ $to }. satırlar katlandı. Açmak için tıklayın.
# Review: PO
hv-editor-fold-name = { $from }–{ $to }. satırları katla
# Review: PO
hv-editor-unfold-name = { $from }–{ $to }. satırların katlamasını aç
# Review: PO
hv-editor-fold-failed = katlanmış bölümde başarısız · satır { $line }
# Review: PO
hv-editor-fold-hidden = { $n } satır katlandı
# Review: PO
hv-editor-fold-done = { $from }–{ $to }. satırlar katlandı
# Review: PO
hv-editor-unfold-done = { $from }–{ $to }. satırların katlaması açıldı

## m3b-t35-grid

# Review: PO
hv-grid-frozen-tip = Donduruldu: yana kaydırınca yerinde kalır
# Review: PO
hv-grid-resize-tip = Boyutlandırmak için sürükleyin · sığdırmak için çift tıklayın
# Review: PO
hv-grid-width = { $px } px
# Review: PO
hv-strip-ranges = { $n } aralık · { $cells } hücre
# Review: PO
hv-grid-freeze-refused = Bu sütunlar dondurulamaz: kaydıracak yer kalmaz. Bölmeyi genişletin ya da daha az sütun dondurun.

## m3b-t36-sort

# Review: PO
hv-grid-sorted-locally = Yerel olarak sıralandı: veritabanı farklı sıralayabilir.
# Review: PO
hv-grid-sorted-locally-more = Yalnızca getirilen { $rows } satır sıralandı.
# Review: PO
hv-grid-clear-sort = Sıralamayı kaldır
# Review: PO
hv-grid-unsorted-tail = Sonda { $rows } yeni satır (sıralanmadı)
# Review: PO
hv-grid-resort = Yeniden sırala
# Review: PO
hv-grid-sorting = { $rows } satır sıralanıyor…
# Review: PO
hv-grid-sorted-tail = Yalnızca getirilen { $sorted } satır sıralandı; sondaki { $rows } yeni satır sıralanmadı.
# Review: PO
hv-grid-sort-cancel = Vazgeç
# Review: PO
hv-grid-sort-tip = Sıralama düzeyi { $lvl }, { $dir }. Metin, sütunun harmanlamasına ({ $collation }) göre karşılaştırılır.
# Review: PO
hv-grid-sort-tip-cp = Sıralama düzeyi { $lvl }, { $dir }. Sütunun { $collation } harmanlamasının burada karşılığı yok; metin kod noktasına göre karşılaştırılır.
# Review: PO
hv-grid-dir-asc = artan
# Review: PO
hv-grid-dir-desc = azalan
# Review: PO
hv-grid-sort-tip-unknown = Sıralama düzeyi { $lvl }, { $dir }. Sütunun harmanlaması burada bilinmiyor; metin kod noktasına göre karşılaştırılır.
# Review: PO
hv-grid-sort-level = Sıralama düzeyi { $lvl }, { $dir }.
# Review: PO
hv-grid-sort-failed = Sıralama bir iç hata yüzünden durdu. Tablo gösterdiği sırayı koruyor.

## m3b-t33-find

# Review: PO
hv-find-placeholder = Bul
# Review: PO
hv-find-match-case = Büyük/küçük harf eşleşsin
# Review: PO
hv-find-words = Tam sözcük
# Review: PO
hv-find-regex = Düzenli ifade
# Review: PO
hv-find-tolerant = Türkçe esnek
# Review: PO
hv-find-tolerant-tip = Türkçe esnek: İ, I, ı ve i birbirini bulur, aksanlar yok sayılır.
# Review: PO
hv-find-count = { $i }/{ $n }
# Review: PO
hv-find-count-more = { $i }/{ $n }+
# Review: PO
hv-find-counting-tip = Sayım sürüyor; belgenin geri kalanı arandıkça sayı artar.
# Review: PO
hv-find-none = Eşleşme yok
# Review: PO
hv-find-invalid = Geçersiz desen
# Review: PO
hv-find-invalid-tip = Düzenli ifade okunamadığı için hiçbir şey aranmıyor: { $error }
# Review: PO
hv-find-prev = Önceki eşleşme
# Review: PO
hv-find-next = Sonraki eşleşme
# Review: PO
hv-find-capped = { $n } eşleşmenin ilk { $max } tanesi seçildi
# Review: PO
hv-find-total = { $n } eşleşme
# Review: PO
hv-find-total-more = { $n }+ eşleşme
# Review: PO
hv-find-no-more = Başka eşleşme yok

## m3b-t37-find-goto

# Review: PO
hv-gfind-placeholder = Görünen değerlerde bul
# Review: PO
hv-gfind-filter = Satırları süz
# Review: PO
hv-gfind-filter-tip = Satırları süz: yalnızca eşleşen satırları göster
# Review: PO
hv-gfind-incomplete = eksik olabilir
# Review: PO
hv-gfind-incomplete-tip = { $n } değer { $size } sınırında kesildi; kesilen kısım aranmaz.
# Review: PO
hv-gfind-filter-note = Getirilen { $rows } satırdan eşleşen { $shown } satır gösteriliyor. Henüz getirilmeyen satırlar süzülmez.
# Review: PO
hv-gfind-show-all = Tüm satırları göster
# Review: PO
hv-gfind-null-tip = Tablonun gösterdiği değerlerde, odaktaki hücreden başlayarak arar. NULL hiçbir zaman eşleşmez.
# Review: PO
hv-gfind-close = Bulmayı kapat
# Review: PO
hv-gfind-select-all = Tüm eşleşmeleri seç
# Review: PO
hv-cmd-grid-select-all-occurrences = Tablo: tüm eşleşmeleri seç
# Review: PO
hv-goto-title = Satıra git
# Review: PO
hv-goto-placeholder = satır ya da satır:sütun
# Review: PO
hv-goto-hint = Satır numarası, ardından isteğe bağlı sütun adı ya da sırası: 1500:city ya da 1500:3
# Review: PO
hv-goto-target = { $row }. satır, { $col } sütunu
# Review: PO
hv-goto-target-row = { $row }. satır
# Review: PO
hv-goto-past-end = { $row }. satır yok: sonuçta { $total } satır var.
# Review: PO
hv-goto-unknown-col = { $name } adında bir sütun yok.
# Review: PO
hv-goto-hidden-col = { $name } sütunu gizli.
# Review: PO
hv-goto-show-col = Gösterip git
# Review: PO
hv-goto-fetch = { $row }. satır henüz getirilmedi. { $more } satır daha getirmek zaman alır; sonucun bellek sınırı yine geçerlidir.
# Review: PO
hv-goto-fetching = { $row }. satıra kadar getiriliyor: { $more } satırın { $done } tanesi
# Review: PO
hv-goto-fetching-note = Esc bunu gizler; getirme şeritte sürer.
# Review: PO
hv-goto-stop = Getirmeyi durdur
# Review: PO
hv-goto-fetch-ok = Getir ve git
# Review: PO
hv-goto-cancel = Vazgeç
# Review: PO
hv-goto-key-go = Git
# Review: PO
hv-goto-not-a-row = Bu bir satır numarası değil.
# Review: PO
hv-goto-past-end-filter = { $row }. satır yok: süzgeç { $total } satır gösteriyor.
# Review: PO
hv-goto-fetch-closed = Getirme { $row }. satırda durdu: imleç kapalı.
# Review: PO
hv-goto-fetch-paused = Getirme { $row }. satırda durdu: sonuç bellek sınırına ulaştı.

## m3b-t38-views

# Review: PO
hv-view-table = Tablo
# Review: PO
hv-view-transposed = Devrik
# Review: PO
hv-view-text = Metin
# Review: PO
hv-view-label = Görünüm
# Review: PO
hv-view-transposed-off = Devrik görünüm en çok { $max } satır gösterir; bu sonuçta { $rows } satır var.
# Review: PO
hv-transposed-col = Sütun
# Review: PO
hv-record = Kayıt
# Review: PO
hv-record-tip = Odaktaki satırı kayıt olarak göster
# Review: PO
hv-record-pos = Satır { $i }/{ $n }
# Review: PO
hv-record-prev = Önceki satır
# Review: PO
hv-record-next = Sonraki satır
# Review: PO
hv-record-show-more = Devamını göster
# Review: PO
hv-record-dup = ({ $i })
# Review: PO
hv-record-dup-tip = Sonuçta { $name } adında { $n } sütun var; bu { $i }. sütun.
# Review: PO
hv-record-label = Odaktaki satırın kaydı
# Review: PO
hv-text-big-title = { $rows } satır metin olarak gösterilsin mi?
# Review: PO
hv-text-big-body = Bu kadar satır için metin görünümünü oluşturmak zaman alır ve yaklaşık { $size } bellek kullanır.
# Review: PO
hv-text-building = Metin görünümü oluşturuluyor: { $rows } satırın { $done } tanesi
# Review: PO
hv-text-cancel = Vazgeç
# Review: PO
hv-text-big-ok = Metin olarak göster
# Review: PO
hv-text-big-back = Tabloya dön
# Review: PO
hv-text-label = Metin tablosu olarak sonuç
# Review: PO
hv-text-copied = { $n } satır kopyalandı
# Review: PO
hv-vv-sub = { $type } · { $row }. satır
# Review: PO
hv-vv-raw = Ham
# Review: PO
hv-vv-pretty = Biçimli
# Review: PO
hv-vv-hex = Onaltılık
# Review: PO
hv-vv-pretty-tip = Yalnızca görünüm: Ham değeri kopyala, sunucunun metnini değiştirmeden kopyalar.
# Review: PO
hv-vv-chars = { $size } · { $n } karakter
# Review: PO
hv-vv-bytes = { $n } bayt
# Review: PO
hv-vv-truncated = Değerin ilk { $shown } kısmı gösteriliyor (toplam { $size }).
# Review: PO
hv-vv-truncated-unknown = Değerin ilk { $shown } kısmı gösteriliyor.
# Review: PO
hv-vv-rerun = { $limit } değer sınırıyla yeniden çalıştır
# Review: PO
hv-vv-rerun-tip = Bu ifadeyi her değer için daha büyük bir sınırla yeniden çalıştırır. Çalıştır ile aynı denetimlerden geçer.
# Review: PO
hv-vv-copy = Ham değeri kopyala
# Review: PO
hv-vv-not-json = Geçerli JSON değil; Biçimli kapalı, değer sunucunun gönderdiği gibi gösteriliyor.
# Review: PO
hv-vv-label = Değer görüntüleyici
# Review: PO
hv-vv-copied = Değer kopyalandı
# Review: PO
hv-vv-copied-cut = Değerin ilk { $shown } kısmı kopyalandı: gerisi getirilmedi.
# Review: PO
hv-vv-rerun-not-read = Daha büyük bir değer sınırıyla yalnızca veri okuyan sorgular yeniden çalıştırılır. Bu ifadeyi düzenleyiciden yeniden çalıştırın.
# Review: PO
hv-vv-rerun-busy = Bu konsolda bir ifade çalışıyor. Bittiğinde yeniden çalıştırın.
