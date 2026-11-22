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
hv-conn-idle-tx = { $name } üzerindeki bir transaction { $minutes } dakikadır açık ve boşta. Commit edilene ya da rollback edilene kadar kilitlerini tutar.
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
hv-conn-lost-tx = Açık transaction'ınız sona erdi ve sunucu commit edilmemiş değişikliklerini rollback etti.
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
hv-conn-gap-tx-ended = { -brand-hacivat }, salt okunur bir bağlantıda salt okunur transaction'ı sonlandıran bir ifadeyi durdurdu ve o oturumu kapattı.
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
hv-conn-ro-warn-temp-privilege = Geçici tablo oluşturabilir; salt okunur transaction'lar bunları engeller.
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
hv-conn-ro-warn-non-transactional-write = Etkileri rollback ile geri alınamayan fonksiyonlar çalıştırabilir.

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
hv-tx-open = Transaction açık · { $n } ifade
# Review: PO
hv-tx-open-tip = BEGIN'den bu yana çalışan ifadeler henüz commit edilmedi. Komut paletinden commit edin ya da rollback edin.
# Review: PO
hv-tx-failed = Transaction başarısız · bitirmek için rollback edin
# Review: PO
hv-tx-failed-tip = Bir hata ya da iptal transaction'ı durdurdu. Sunucu artık yalnızca rollback kabul eder.
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
hv-console-ro-pending = { $name }, açık transaction'ı commit edildiğinde ya da rollback edildiğinde salt okunur olur.
# Review: PO
hv-console-ro-pending-short = Bu transaction'dan sonra salt okunur
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
hv-cmd-tx-commit = Transaction'ı commit et
# Review: PO
hv-cmd-tx-rollback = Transaction'ı rollback et
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
hv-tx-close-title = { $name } üzerinde { $n } ifadelik açık bir transaction var
# Review: PO
hv-tx-close-body = Bu konsolu kapatmak transaction'ını sonlandırır. Commit değişiklikleri korur, rollback onları atar.
# Review: PO
hv-tx-restart-body = Yeniden başlatmak oturumu sonlandırır. Commit değişiklikleri korur, rollback onları atar.
# Review: PO
hv-tx-commit-restart = Commit et ve yeniden başlat
# Review: PO
hv-tx-rollback-restart = Rollback et ve yeniden başlat
# Review: PO
hv-tx-release-body = Konsolun bağlantısı değişmek üzere; bu, transaction'ı sonlandırır. Commit değişiklikleri korur, rollback onları atar.
# Review: PO
hv-tx-commit-continue = Commit et ve devam et
# Review: PO
hv-tx-rollback-continue = Rollback et ve devam et
# Review: PO
hv-tx-quit-body = Çıkmak oturumu sonlandırır. Commit değişiklikleri korur, rollback onları atar.
# Review: PO
hv-tx-commit-close = Commit et ve kapat
# Review: PO
hv-tx-rollback-close = Rollback et ve kapat
# Review: PO
hv-tx-commit-quit = Commit et ve çık
# Review: PO
hv-tx-rollback-quit = Rollback et ve çık
# Review: PO
hv-tx-cancel = Vazgeç
# Review: PO
hv-tx-failed-title = { $name } üzerindeki transaction başarısız oldu
# Review: PO
hv-tx-failed-body = Bir hata ya da iptal onu durdurdu; artık yalnızca rollback edilebilir. İçindeki { $n } ifade korunmayacak.
# Review: PO
hv-tx-end-failed = { $name } üzerindeki transaction sona ermedi; hiçbir şey kapatılmadı.
# Review: PO
hv-tx-failed-by-stop = İfadeyi durdurmak { $name } üzerindeki transaction'ı başarısız kıldı. Artık yalnızca rollback onu sonlandırabilir.
# Review: PO
hv-exec-pending-read-only = Hiçbir şey gönderilmedi: bu konsol, açık transaction'ı sona erdiğinde salt okunur olur. Önce commit edin ya da rollback edin.
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
hv-tx-chip-tip = Transaction kipi: { $mode }
# Review: PO
hv-tx-menu-title = Transaction kipi
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
hv-tx-menu-reason-open = Önce commit edin ya da rollback edin
# Review: PO
hv-tx-menu-prod-note = Üretim konsolları Manuel kipte başlar.
# Review: PO
hv-tx-chip-ro-reason = Salt okunur bir oturum her ifadeyi tek başına çalıştırır; bu yüzden kipi her zaman Otomatik commit'tir.
# Review: PO
hv-tx-badge = Transaction · { $n } ifade · { $age }
# Review: PO
hv-tx-badge-short = Transaction · { $n } ifade
# Review: PO
hv-tx-badge-failed = Transaction başarısız
# Review: PO
hv-tx-badge-tip = Henüz commit edilmedi: diğer oturumlar bu değişiklikleri görmez ve dokunulan satırlar kilitli kalır.
# Review: PO
hv-tx-badge-failed-tip = Bir hata ya da iptal transaction'ı durdurdu. Sunucu artık yalnızca rollback kabul eder.
# Review: PO
hv-tx-badge-commit = Commit et
# Review: PO
hv-tx-badge-rollback = Rollback et
# Review: PO
hv-tx-segment-tip = Commit, rollback ve transaction kipi için tıklayın.
# Review: PO
hv-tx-idle-title = { $name } üzerinde { $age } süredir açık bir transaction var
# Review: PO
hv-tx-idle-body = Diğer oturumları engelleyen kilitler tutuyor olabilir.
# Review: PO
hv-tx-idle-commit = Commit et
# Review: PO
hv-tx-idle-rollback = Rollback et
# Review: PO
hv-tx-idle-keep = Açık bırak
# Review: PO
hv-tx-lost-maybe = Açık bir transaction'ı olmuş olabilir; öyleyse sunucu commit edilmemiş değişikliklerini rollback etti.
# Review: PO
hv-conn-sessions = Oturumlar
# Review: PO
hv-conn-single-session = Tüm konsollar için tek oturum kullan
# Review: PO
hv-conn-single-session-sub = Bu bağlantının konsolları tek oturumu paylaşır: transaction'ını, değişkenlerini ve geçici tablolarını.
# Review: PO
hv-conn-budget-single = Tüm konsolların paylaştığı tek oturum
# Review: PO
hv-console-shared = Paylaşılan oturum
# Review: PO
hv-console-shared-tip = { $name } bağlantısının tüm konsolları bu oturumu kullanır: burada açılan bir transaction hepsinde açıktır.

## m3b-t22-tx-ui-r1

# Review: PO
hv-tx-idle-waits = Konsol, çalıştırmasıyla ilgili bir yanıt bekliyor. Yanıtlandıktan sonra commit edin ya da rollback edin.
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
hv-confirm-class-tx = Transaction'ı denetler
# Review: PO
hv-confirm-class-unknown = Bilinmiyor: Hacivat ne yaptığını anlayamıyor
# Review: PO
hv-confirm-mysql-ddl = MySQL DDL'i hemen commit eder; bu yüzden rollback edilemez.
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
hv-confirm-execute-all-scope = Tümünü çalıştır, bu bağlantıda ({ $env }) sorulacak kalan { $n } ifadeyi de sormadan çalıştırır. Adının yazılması gereken bir ifade yine sorar.

# Review: PO
hv-commit-title = { $name } üzerinde { $rows } satır değişti
# Review: PO
hv-commit-title-unknown = İfade { $name } üzerinde veriyi değiştirdi
# Review: PO
hv-commit-body-own = Hacivat bu ifadeyi kendi açtığı bir transaction'da çalıştırdı. Commit edene kadar diğer oturumlar değişikliği görmez.
# Review: PO
hv-commit-locks = Yanıt beklenirken satırlar kilitli kalır. Yanıt gelmezse Hacivat 5 dakika sonra rollback eder.
# Review: PO
hv-commit-commit = Commit et
# Review: PO
hv-commit-rollback = Rollback et
# Review: PO
hv-commit-countdown = { $time } sonra rollback edilir
# Review: PO
hv-commit-sp-title = Transaction'ınızın içinde { $rows } satır değişti
# Review: PO
hv-commit-sp-title-unknown = İfade, transaction'ınızın içinde veriyi değiştirdi
# Review: PO
hv-commit-sp-keep-line = Koru: değişiklik açık transaction'ınıza katılır. Henüz hiçbir şey commit edilmez.
# Review: PO
hv-commit-sp-undo-line = İfadeyi geri al: yalnızca bu ifadenin değişikliği kaldırılır; transaction'ınız açık kalır.
# Review: PO
hv-commit-sp-locks = Yanıt gelmezse Hacivat 5 dakika sonra ifadeyi geri alır ve transaction'ınızı açık bırakır.
# Review: PO
hv-commit-keep = Koru
# Review: PO
hv-commit-undo = İfadeyi geri al
# Review: PO
hv-commit-sp-countdown = İfade { $time } sonra geri alınır
# Review: PO
hv-commit-dropped-title = { $name } bağlantısı koptu
# Review: PO
hv-commit-dropped-body = Sunucu bu değişiklikleri rollback etti. Hiçbir şey commit edilmedi.
# Review: PO
hv-commit-dropped-sp-body = Sunucu açık transaction'ınızı rollback etti: bu ifade ve ondan önceki { $n } ifade. Hiçbir şey commit edilmedi.
# Review: PO
hv-commit-dropped-sp-body-unknown = Sunucu, bu ifade de içinde olmak üzere açık transaction'ınızı rollback etti. Hiçbir şey commit edilmedi.
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
hv-cancel-own-tx = Bu konsolun açık transaction'ı ({ $n } ifade) rollback edilir.
# Review: PO
hv-cancel-own-tx-maybe = Bu konsolun açık bir transaction'ı olabilir; öyleyse rollback edilir.
# Review: PO
hv-cancel-own-tx-hacivat = Hacivat'ın bu konsolda açtığı transaction (yanıtınızı bekleyen bir değişiklik ya da açık bir imleç) rollback edilir.
# Review: PO
hv-cancel-other-tx-hacivat = { $console } konsolunda da Hacivat'ın açtığı bir transaction var; sunucu onu rollback eder.
# Review: PO
hv-cancel-disconnect = Bağlantıyı kes
# Review: PO
hv-cancel-disconnect-line-all = “Bağlantıyı kes”, { $name } için buradaki tüm bağlantıları kapatır; ifade sunucuda çalışmayı sürdürebilir.
# Review: PO
hv-cancel-disconnect-line-one = “Bağlantıyı kes” yalnızca bu konsolun oturumunu ({ $console }) kapatır; ifade sunucuda çalışmayı sürdürebilir.
# Review: PO
hv-cancel-disconnect-line-local = “Bağlantıyı kes”, bu bağlantının konsollarının paylaştığı { $name } dosyası bağlantısını kapatır; SQLite, bağlantı kapanınca ifadeyi durdurur.
# Review: PO
hv-cancel-other-tx = { $console } konsolunda da açık bir transaction var ({ $n } ifade); sunucu onu rollback eder.
# Review: PO
hv-cancel-other-tx-maybe = { $console } konsolunun da açık bir transaction'ı olabilir; öyleyse sunucu onu rollback eder.
# Review: PO
hv-cancel-terminate-failed = Oturum sonlandırılamadı. Hacivat iptali beklemeyi sürdürüyor.
# Review: PO
hv-editor-mark-escalated = iptal yanıtlanmadı · { $duration }
# Review: PO
hv-errbar-failed = { $i }. ifade başarısız oldu ({ $n } ifadeden)
# Review: PO
hv-errbar-aborted = { $i }. ifade başarısız oldu ({ $n } ifadeden); transaction da başarısız oldu
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
hv-errbar-rollback-continue = Rollback et ve devam et
# Review: PO
hv-errbar-rollback-continue-tip = Açık transaction'ınızı ({ $n } ifade) rollback eder, sonra { $next }. ifadeden yeni bir transaction'da devam eder.
# Review: PO
hv-errbar-rollback-continue-tip-auto = Açık transaction'ınızı ({ $n } ifade) rollback eder, sonra { $next }. ifadeden devam eder; her ifade çalışınca commit edilir.

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
hv-editor-mark-awaiting = Commit ya da rollback bekleniyor
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

## m3b-t34-editor

# Review: PO
hv-gutter-run = Bu ifadeyi çalıştır
# Review: PO
hv-gutter-last-runs = Son çalıştırmalar
# Review: PO
hv-gutter-never-run = Henüz çalıştırılmadı.
# Review: PO
hv-gutter-same-path = Çalıştır ile aynı denetimlerden geçer.
# Review: PO
hv-eol-mixed = Karışık satır sonları
# Review: PO
hv-eol-mixed-tip = Satırlar birden çok biçimde bitiyor ({ $mix }). Eşitlemek için tıklayın.
# Review: PO
hv-eol-counts = Satır sonları: LF { $lf }, CRLF { $crlf }
# Review: PO
hv-eol-to = Hepsini { $ending } yap
# Review: PO
hv-eol-most = çoğu satır
# Review: PO
hv-history-recall = Geçmişten: ifade { $i }/{ $n } · { $older } daha eski · { $newer } daha yeni
# Review: PO
hv-editor-mark-rolled-back = geri alındı
# Review: PO
hv-editor-preview-as-one = tek parça · { $n } ifade
# Review: PO
hv-eol-counts-cr = Satır sonları: LF { $lf }, CRLF { $crlf }, CR { $cr }
# Review: PO
hv-gutter-run-line = { $line }. satırdaki ifadeyi çalıştır

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

## m3b-t42-settings

# Review: PO
hv-settings-nav-editor = Düzenleyici
# Review: PO
hv-settings-nav-execution = Çalıştırma
# Review: PO
hv-settings-nav-results = Sonuçlar
# Review: PO
hv-settings-nav-history = Geçmiş
# Review: PO
hv-settings-section-editor = DÜZENLEYİCİ
# Review: PO
hv-settings-section-execution = ÇALIŞTIRMA
# Review: PO
hv-settings-section-results = SONUÇLAR
# Review: PO
hv-settings-section-history = GEÇMİŞ
# Review: PO
hv-settings-on = Açık
# Review: PO
hv-settings-off = Kapalı
# Review: PO
hv-settings-auto = Otomatik
# Review: PO
hv-settings-tab-size = Sekme genişliği
# Review: PO
hv-settings-insert-spaces = Boşlukla girintile
# Review: PO
hv-settings-copy-line = Seçim yokken satırı kopyala
# Review: PO
hv-settings-caret-blink = Yanıp sönen imleç
# Review: PO
hv-settings-find-tolerant = Türkçe esnek arama
# Review: PO
hv-settings-find-tolerant-note = Otomatik: dil ya da bölge biçimi Türkçe olduğunda açık. Bul çubuğundaki düğme de bu ayarı değiştirir.
# Review: PO
hv-settings-confirm-unsafe-where = Üretim dışında WHERE içermeyen UPDATE ya da DELETE için onay iste
# Review: PO
hv-settings-confirm-unsafe-where-note = Üretim her zaman sorar ve tablonun adını yazdırır.
# Review: PO
hv-settings-error-policy = Bir ifade başarısız olunca ne yapılacağını sor
# Review: PO
hv-settings-error-policy-note = Kapalı: birden çok ifadelik çalıştırma ilk hatada durur.
# Review: PO
hv-settings-max-value = Hücre başına saklanan en büyük değer
# Review: PO
hv-settings-max-value-note = 64 KB ile 64 MB arası. Daha büyük değerler kesilir; değer görüntüleyici en büyük sınırla yeniden çalıştırabilir.
hv-settings-max-value-amount = { $n }
# Review: PO
hv-settings-max-value-clamped = { $typed } izin verilen en büyük değerin üstünde; { $max } yapıldı.
# Review: PO
hv-settings-max-value-clamped-low = { $typed } izin verilen en küçük değerin altında; { $min } yapıldı.
hv-settings-unit-kb = KB
hv-settings-unit-mb = MB
# Review: PO
hv-settings-raw-display = Tablo değerlerini ham göster
# Review: PO
hv-settings-raw-display-note = Sayılar ve tarihler bölgesel biçim olmadan, sunucunun gönderdiği gibi. Kopyalama her zaman hamdır.
# Review: PO
hv-settings-history = Sorgu geçmişini kaydet
# Review: PO
hv-settings-history-note = Bağlantı başına en çok 10.000 ifade 90 gün saklanır; tanınan parolalar ve gizli bilgiler maskelenir.

## m3c-int-fixes

# Review: PO
hv-goto-fetch-rerun = Getirme durdu: sonuç yeniden çalıştırıldı.

## m3b-t41-chrome

# Review: PO
hv-tx-chip-auto-short = Otomatik
# Review: PO
hv-console-waiting-l1 = Bağlantı bekleniyor
# Review: PO
hv-console-waiting-l2 = kullanımda: { $inUse }/{ $max }
# Review: PO
hv-console-button-keys = { $label } · { $keys }
# Review: PO
hv-editor-placeholder = SQL'i buraya yazın. { $key }, imlecin bulunduğu ifadeyi çalıştırır.

## m3b-t39-grid-menu

# Review: PO
hv-gmenu-copy = Kopyala
# Review: PO
hv-gmenu-copy-headers = Başlıklarla kopyala
# Review: PO
hv-gmenu-copy-rows = Satırları kopyala
# Review: PO
hv-gmenu-copy-name = Sütun adını kopyala
# Review: PO
hv-gmenu-open-value = Değer görüntüleyiciyi aç
# Review: PO
hv-gmenu-sort-asc = Artan sırala
# Review: PO
hv-gmenu-sort-desc = Azalan sırala
# Review: PO
hv-gmenu-add-sort = { $lvl }. sıralama düzeyi olarak ekle
# Review: PO
hv-gmenu-freeze = Buraya kadar dondur
# Review: PO
hv-gmenu-unfreeze = Dondurmayı kaldır
# Review: PO
hv-gmenu-hide = Sütunu gizle
# Review: PO
hv-gmenu-show-all = Tüm sütunları göster
# Review: PO
hv-gmenu-autofit = Genişliği sığdır
# Review: PO
hv-gmenu-raw = Ham görünüm
# Review: PO
hv-gmenu-raw-hint = bu sütun
# Review: PO
hv-gmenu-goto = Satıra git…
# Review: PO
hv-gmenu-record = Kayıt olarak göster
# Review: PO
hv-gmenu-why-one-cell = Tek hücre seçin
# Review: PO
hv-gmenu-why-not-sorted = Sıralı değil
# Review: PO
hv-gmenu-why-no-hidden = Gizli sütun yok
# Review: PO
hv-gmenu-why-ranges = Aralıklar hizalı değil
# Review: PO
hv-gmenu-why-last = Görünen son sütun
# Review: PO
hv-gmenu-label = Tablo menüsü

## m3c-po

# Review: PO
hv-cmd-editor-use-selection-for-find = Seçimi bulmada kullan

## m4-t01b-ui

# Surface 1 (Explorer tree).
# Review: PO
hv-explorer-tree-label = Veritabanı gezgini
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-tables = TABLOLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-views = GÖRÜNÜMLER
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-matviews = SOMUT GÖRÜNÜMLER
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-functions = FONKSİYONLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-procedures = PROSEDÜRLER
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-sequences = SEKANSLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-types = TİPLER
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-events = OLAYLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-columns = SÜTUNLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-keys = ANAHTARLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-foreign-keys = YABANCI ANAHTARLAR
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-indexes = İNDEKSLER
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-checks = DENETİMLER
# Surface 1 (Explorer tree). A kind-group header: small caps, pre-cased here, never cased in code.
# Review: PO
hv-explorer-kind-triggers = TETİKLEYİCİLER
# Surface 1 (Explorer tree). The Built-in group of a database.
# Review: PO
hv-explorer-builtin = Yerleşik
# Surface 1 (Explorer tree). The Built-in group of a database.
# Variables:
#   $version (String) - the bundled catalog's server version, e.g. "17".
# Review: PO
hv-explorer-builtin-badge = sürüm { $version }
# Surface 1 (Explorer tree). The Built-in group of a database.
# Variables:
#   $version (String) - the bundled catalog's server version, e.g. "17".
# Review: PO
hv-explorer-builtin-tip = Yerleşik katalog (sürüm { $version }): sunucunun kendi nesneleri; Hacivat ile gelir, bağlantı olmadan okunur.
# Surface 1 (Explorer tree). The database row's schema scope count (N of M).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-explorer-scope-count = { $n } / { $total }
# Surface 1 (Explorer tree). The database row's schema scope count (N of M).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-explorer-scope-count-tip = { $total } şemadan { $n } tanesi kapsamda. Kapsamı düzenle… hangilerinin okunacağını değiştirir.
# Surface 1 (Explorer tree).
# Review: PO
hv-explorer-add-scope = Kapsama ekle
# Surface 1 (Explorer tree).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-explorer-out-of-scope-tip = { $name } bu bağlantının kapsamında değil, bu yüzden okunmuyor.
# Surface 1 (Explorer tree).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-explorer-out-of-scope-db-tip = { $name } bu bağlantının kapsamında değil, bu yüzden okunmuyor. Okumak bir bağlantı daha açar.
# Surface 1 (Explorer tree).
# Variables:
#   $rows (Number) - the server's row estimate, e.g. 48213.
# Review: PO
hv-explorer-row-estimate-tip = Yaklaşık { $rows } satır (sunucunun tahmini)
# Surface 1 (Explorer tree).
# Review: PO
hv-explorer-loads-on-open = Açtığınızda yüklenir
# Surface 2 (Loading states).
# Review: PO
hv-explorer-loading = Yükleniyor…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
# Review: PO
hv-explorer-phase-names = Adlar okunuyor…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
# Review: PO
hv-explorer-phase-columns = Sütunlar okunuyor…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
# Review: PO
hv-explorer-phase-constraints = Anahtarlar ve indeksler okunuyor…
# Surface 2 (Loading states). A phase row's label; the count follows in mono (hv-explorer-progress-count).
# Review: PO
hv-explorer-phase-sources = Tanımlar okunuyor…
# Surface 2 (Loading states). Progress of a pass in the explorer (surface 14: explorer only).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-explorer-progress-count = { $n }/{ $total }
# Surface 2 (Loading states).
# Review: PO
hv-explorer-waiting-pass = Sütunlar bekleniyor
# Surface 2 (Loading states). The partial-load badge and its tooltip.
# Review: PO
hv-explorer-partial = eksik
# Surface 2 (Loading states). The partial-load badge and its tooltip.
# Review: PO
hv-explorer-partial-title = Bazı nesneler okunamadı
# Surface 2 (Loading states). The partial-load badge and its tooltip.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-explorer-partial-body = Bu kullanıcı { $schema } içindeki { $n } fonksiyonun tanımını okuyamıyor. Adlarıyla listelenirler; tanımları eksiktir.
# Surface 2 (Loading states). A namespace whose catalog query was refused.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-explorer-failed = { $name } okunamadı
# Surface 2 (Loading states). A namespace whose catalog query was refused.
# Review: PO
hv-explorer-failed-body = Sunucu katalog sorgusunu reddetti. Yeniden dene ile tekrar okunur.
# Surface 2 (Loading states).
# Review: PO
hv-explorer-retry = Yeniden dene
# Surface 3 (Cached and offline). A profile shown from its cache while offline.
# Variables:
#   $date (DateTime) - when the cache was saved, e.g. 2026-11-17 14:05.
# Review: PO
hv-explorer-cached = { DATETIME($date, dateStyle: "medium", timeStyle: "short") } tarihli önbellekteki üst veri gösteriliyor
# Surface 3 (Cached and offline). A profile shown from its cache while offline.
# Review: PO
hv-explorer-cached-meta = önbellek
# Surface 3 (Cached and offline). A profile shown from its cache while offline.
# Variables:
#   $date (DateTime) - when the cache was saved, e.g. 2026-11-17 14:05.
# Review: PO
hv-explorer-cached-tip = Bağlı değil. Ağaç, { DATETIME($date, dateStyle: "medium", timeStyle: "short") } tarihinde kaydedilen kopyadır; sunucuyla karşılaştırmak için bağlanın.
# Surface 3 (Cached and offline).
# Review: PO
hv-explorer-stale = güncel değil
# Surface 3 (Cached and offline).
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-explorer-stale-tip = { $schema } okunduktan sonra sunucuda değişti. Değişiklikleri yüklemek için yenileyin.
# Surface 3 (Cached and offline).
# Review: PO
hv-explorer-encrypted-tip = Önbellekteki üst veri şifreli. Bu bağlantıyı açmak anahtarını Anahtar Zinciri'nden okur.
# Surface 3 (Cached and offline).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-explorer-memory-only = Anahtar Zinciri kullanılamıyor; { $name } üst verisi uygulamadan çıkana dek yalnız bellekte tutulur.
# Surface 4 (Filter field). The explorer filter field.
# Review: PO
hv-explorer-filter-objects = Nesnelerde ara
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-explorer-filter-count = { $n } eşleşme
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-explorer-filter-group-count = { $n } / { $total }
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-explorer-filter-unloaded = Yalnız yüklenmiş adlarda arar. { $n } tablonun sütunları henüz yüklenmedi.
# Surface 4 (Filter field). The explorer filter field.
# Review: PO
hv-explorer-filter-none = Eşleşen nesne yok
# Surface 4 (Filter field). The explorer filter field.
# Variables:
#   $query (String) - the filter text as typed, e.g. "müş".
# Review: PO
hv-explorer-filter-none-body = Yüklenmiş adlardan hiçbiri “{ $query }” ile eşleşmiyor.
# Surface 4 (Filter field). The explorer filter field.
# Review: PO
hv-explorer-filter-clear = Filtreyi temizle
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-refresh = Yenile
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-edit-scope = Kapsamı düzenle…
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-level = Okuma düzeyi
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-clear-cache = Önbellekteki üst veriyi temizle…
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-open-data = Veriyi aç
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-view-ddl = DDL'i göster
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-view-table-ddl = Tablonun DDL'ini göster
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-copy-name = Tam adı kopyala
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-copy-plain = Adı kopyala
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-hide-like = Benzer nesneleri gizle
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-show-hidden = Gizli nesneleri göster
# Surface 5 (Context menus).
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $pattern (String) - the view filter's pattern, e.g. "yedek_fatura_*".
# Review: PO
hv-explorer-hidden-tip = { $n } nesne bir görünüm filtresiyle gizlendi ({ $pattern }). Bu grubun menüsündeki Gizli nesneleri göster filtreyi kaldırır.
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-copy-ddl = DDL'i kopyala
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-modify-table = Tabloyu değiştir…
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-modify-view = Görünümü değiştir…
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-modify-column = Sütunu değiştir…
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-modify-tip = Sonraki bir sürümde gelecek
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-remove-scope = Kapsamdan çıkar
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-expand-all = Tüm alt öğeleri aç
# Surface 5 (Context menus). The tree-wide command (palette, ⌘←); node menus use hv-explorer-menu-collapse-children.
# Review: PO
hv-explorer-menu-collapse-all = Tümünü kapat
# Surface 5 (Context menus).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-explorer-copied = { $name } kopyalandı
# Surface 5 (Context menus).
# Variables:
#   $conn (String) - the connection name, e.g. "orders-dev".
# Review: PO
hv-explorer-open-data-note = { $conn } üzerinde bir konsol açar ve şunu çalıştırır:
# Surface 5 (Context menus). An explorer context-menu item.
# Review: PO
hv-explorer-menu-collapse-children = Tüm alt öğeleri kapat
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-auto = Otomatik
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-names = Yalnız adlar
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-structure = Yapı
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-sources = Yapı ve kaynaklar
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Variables:
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
# Review: PO
hv-explorer-level-auto-hint = { $level ->
        [names] şu an Yalnız adlar
        [sources] şu an Yapı ve kaynaklar
       *[structure] şu an Yapı
    }
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-names-hint = en hızlısı, sütunsuz
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-structure-hint = sütunlar, anahtarlar, indeksler
# Surface 6 (Introspection level). The introspection level list (menu and pill).
# Review: PO
hv-explorer-level-sources-hint = ve görünüm, fonksiyon metinleri
# Surface 6 (Introspection level). The level pill's word.
# Review: PO
hv-explorer-pill-names = Adlar
# Surface 6 (Introspection level). The level pill's word.
# Review: PO
hv-explorer-pill-structure = Yapı
# Surface 6 (Introspection level). The level pill's word.
# Review: PO
hv-explorer-pill-sources = Kaynaklar
# Surface 6 (Introspection level). The level pill's tooltip and accessible name: one whole sentence per level.
# Variables:
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
# Review: PO
hv-explorer-pill-tip-explicit = { $level ->
        [names] Okuma düzeyi: Yalnız adlar, burada ayarlandı. Alttaki şemalar bunu devralır.
        [sources] Okuma düzeyi: Yapı ve kaynaklar, burada ayarlandı. Alttaki şemalar bunu devralır.
       *[structure] Okuma düzeyi: Yapı, burada ayarlandı. Alttaki şemalar bunu devralır.
    }
# Surface 6 (Introspection level). The level pill's tooltip and accessible name: one whole sentence per level.
# Variables:
#   $parent (String) - the database or schema the level comes from, e.g. "orders".
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
# Review: PO
hv-explorer-pill-tip-inherited = { $level ->
        [names] Okuma düzeyi: Yalnız adlar; { $parent } öğesinden devralındı.
        [sources] Okuma düzeyi: Yapı ve kaynaklar; { $parent } öğesinden devralındı.
       *[structure] Okuma düzeyi: Yapı; { $parent } öğesinden devralındı.
    }
# Surface 6 (Introspection level). The level pill's tooltip and accessible name: one whole sentence per level.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $level (String) - the level code, a selector: "names", "structure" or "sources".
# Review: PO
hv-explorer-pill-tip-auto = { $level ->
        [names] Okuma düzeyi: Yalnız adlar; { $n } nesne için otomatik seçildi.
        [sources] Okuma düzeyi: Yapı ve kaynaklar; { $n } nesne için otomatik seçildi.
       *[structure] Okuma düzeyi: Yapı; { $n } nesne için otomatik seçildi.
    }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-section = Şemalar
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-scope-summary = { $total } şemadan { $n }
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-scope-summary-mysql = { $total } veritabanının tümü
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-current-db = geçerli veritabanı
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-other-db-hint = ayrı bağlantı gerekir
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-pattern = Desen
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-pattern-hint = @ geçerli veritabanı ya da şema, * tümü, {"{"}…{"}"} bir düzenli ifade içerir; | ifadeleri ayırır.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $pos (Number) - the 1-based character position of the error, e.g. 20.
# Review: PO
hv-scope-pattern-error = “:” işaretinden sonra bir şema adı bekleniyordu ({ NUMBER($pos, useGrouping: "false") }. karakter).
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-builtin-note = Yerleşik şemalar paketle gelen katalogdan okunur, burada listelenmez.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-mysql-note = Görebildiğiniz her veritabanı listelenir. Her biri ilk açtığınızda yüklenir.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-all-dbs = Tüm veritabanları
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-all-dbs-title = Bu sunucudaki tüm veritabanları okunsun mu?
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-scope-all-dbs-body = Bu sunucuda { $n } veritabanı var. Hepsini okumak her veritabanı için bir bağlantı açar ve büyük bir sunucuda zaman alabilir.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-scope-all-dbs-yes = { $n } veritabanının tümünü oku
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-all-dbs-no = Geçerli veritabanında kal
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-encrypt = Önbellekteki üst veriyi şifrele
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-encrypt-sub = Kaydedilen katalog, Anahtar Zinciri'nde tutulan bir anahtarla şifrelenir. Üretimde varsayılan olarak açıktır.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-scope-encrypt-confirm-on = Bunu açmak { $name } için önbellekteki üst veriyi siler. Bir sonraki bağlanışta şifreli olarak yeniden okunur.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-scope-encrypt-confirm-off = Bunu kapatmak { $name } için şifreli önbelleği siler. Bir sonraki bağlanıştan sonra yalnız hesabınızın okuyabildiği düz bir dosyaya kaydedilir.
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-encrypt-apply = Önbelleği sil ve şifrele
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-encrypt-apply-off = Önbelleği sil ve şifrelemeyi bırak
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-encrypt-keep = Olduğu gibi bırak
# Surface 7 (Scope editor). The connection dialog's Schemas section.
# Review: PO
hv-scope-connect-to-list = Şemaları listelemek için bağlanın. Neyin okunacağına yine aşağıdaki desen karar verir.
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-metadata-mass-title = { $schema } içindeki { $count } nesne artık görünmüyor
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Review: PO
hv-metadata-mass-body = Silinmiş olabilirler ya da bu kullanıcı onlara erişimini kaybetmiş olabilir. Önbellekten kaldırmak sunucuda hiçbir şeyi değiştirmez.
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-metadata-mass-more = ve { $n } tane daha
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Review: PO
hv-metadata-mass-keep = Önbellekte tut
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Review: PO
hv-metadata-mass-remove = Önbellekten kaldır
# Surface 8 (Mass-deletion confirmation). The mass-deletion confirmation (D-039).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-metadata-mass-pending-tip = Son arka plan denetiminde { $count } nesne kayboldu. İncelemek için { $schema } şemasını yenileyin; hiçbir şey kaldırılmadı.
# Surface 8 (Mass-deletion confirmation).
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-metadata-ns-dropped = { $schema } artık sunucuda yok, bu yüzden gezginden kaldırıldı.
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
# Review: PO
hv-metadata-refresh-ask = Gezgin, değişen { $count } nesneyi gösterecek şekilde yenilensin mi?
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Variables:
#   $console (String) - the console's title, e.g. "orders-dev".
# Review: PO
hv-metadata-refresh-ask-body = { $console } içindeki betiğiniz bunları değiştirdi.
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Review: PO
hv-metadata-refresh = Yenile
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Review: PO
hv-metadata-refresh-always = Her zaman yenile
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Review: PO
hv-metadata-refresh-not-now = Şimdi değil
# Surface 9 (Refresh after DDL). Refresh after DDL inside a transaction (the pending stamp).
# Review: PO
hv-metadata-after-commit = COMMIT sonrası yenilenir
# Surface 9 (Refresh after DDL). Refresh after DDL inside a transaction (the pending stamp).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
# Review: PO
hv-metadata-after-commit-tip = Bu transaction'daki { $count } değişiklik COMMIT'i bekliyor. ROLLBACK bunları atar ve gezgin olduğu gibi kalır.
# Surface 9 (Refresh after DDL). Refresh after DDL: dynamic SQL.
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-metadata-broad = { $schema } tamamen yenileniyor
# Surface 9 (Refresh after DDL). Refresh after DDL: dynamic SQL.
# Variables:
#   $what (String) - the SQL keyword that made the change opaque, e.g. "EXECUTE".
# Review: PO
hv-metadata-broad-body = Betik dinamik SQL çalıştırdı ({ $what }); Hacivat hangi nesnelerin değiştiğini bilemiyor.
# Surface 9 (Refresh after DDL). Refresh after DDL.
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
# Review: PO
hv-metadata-refreshed = Gezgin { $schema } içinde { $count } nesneyi yeniledi
# Surface 9 (Refresh after DDL).
# Variables:
#   $count (Number) - a count of objects, e.g. 312.
# Review: PO
hv-metadata-log-pending = Gezgin yenilemesi COMMIT'i bekliyor ({ $count } nesne)
# Surface 10 (DDL view). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-tab-title = { $name } · DDL
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-qualification = Nitelendirme
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-qual-auto = Otomatik
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-qual-never = Hiçbir zaman
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-qual-always = Her zaman
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-constraints = Kısıtlar
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-constraints-inside-column = Sütunun içinde
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-constraints-inside-table = Tablonun içinde
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-constraints-after-table = Tablodan sonra
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-server = Sunucunun verdiği tanımlar
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-grants = Yetkiler
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-owner = Sahip
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-options = Seçenekler
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-copy = Kopyala
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-open-console = Konsolda aç
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-refresh = Yenile
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-source-generated = Katalogdan üretildi
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-source-server = Sunucudan gelen metin
# Surface 10 (DDL view). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-copied = { $name } DDL'i kopyalandı
# Surface 10 (DDL view). One whole sentence per kind ($kind selector); never a kind word inserted.
# Variables:
#   $kind (String) - the object kind code, a selector, e.g. "index".
# Review: PO
hv-ddl-view-unsupported = { $kind ->
        [domain] Hacivat domain'lerin DDL'ini henüz gösteremiyor.
        [extension] Hacivat eklentilerin DDL'ini henüz gösteremiyor.
        [operator] Hacivat işleçlerin DDL'ini henüz gösteremiyor.
        [policy] Hacivat satır güvenliği ilkelerinin DDL'ini henüz gösteremiyor.
        [rule] Hacivat kuralların DDL'ini henüz gösteremiyor.
        [type] Hacivat tiplerin DDL'ini henüz gösteremiyor.
       *[other] Hacivat bu tür nesnelerin DDL'ini henüz gösteremiyor.
    }
# Surface 10 (DDL view). One whole sentence per kind ($kind selector); never a kind word inserted.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-unsupported-body = { $name } gezginde kalır; Tam adı kopyala çalışır.
# Surface 10 (DDL view). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-loading = { $name } tanımı okunuyor…
# Surface 10 (DDL view). The DDL view tab.
# Review: PO
hv-ddl-view-grants-partial = Bazı yetkiler okunamadı; liste eksik olabilir.
# Surface 11 (Stale object). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-stale = { $name } yüklendikten sonra sunucuda değişti
# Surface 11 (Stale object). The DDL view tab.
# Review: PO
hv-ddl-view-reload = Yeniden yükle
# Surface 11 (Stale object). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-gone = { $name } artık sunucuda yok
# Surface 11 (Stale object). The DDL view tab.
# Review: PO
hv-ddl-view-gone-body = Aşağıdaki metin en son yüklenen metindir.
# Surface 11 (Stale object). The DDL view tab.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-unknown = Hacivat, { $name } yüklendikten sonra sunucuda değişip değişmediğini denetleyemedi
# Surface 12 (Invisible characters and broken objects). The invisible-characters badge and its tooltip.
# Review: PO
hv-explorer-invisible = görünmez karakter
# Surface 12 (Invisible characters and broken objects). The invisible-characters badge and its tooltip.
# Variables:
#   $what (String) - the SQL keyword that made the change opaque, e.g. "EXECUTE".
# Review: PO
hv-explorer-invisible-tip = Bu ad görünmez bir karakter içeriyor: { $what }. Kopyalama ve DDL onu aynen korur.
# Surface 12 (Invisible characters and broken objects). One whole sentence per kind ($kind selector); the name stays an isolated argument.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
#   $kind (String) - the object kind code, a selector, e.g. "index".
# Review: PO
hv-explorer-broken = { $kind ->
        [check] { $name } denetim kısıtı, çözümlenemeyen bir sütuna başvuruyor
        [foreign-key] { $name } yabancı anahtarı, çözümlenemeyen bir sütuna başvuruyor
        [index] { $name } indeksi, çözümlenemeyen bir sütuna başvuruyor
        [trigger] { $name } tetikleyicisi, çözümlenemeyen bir sütuna başvuruyor
       *[other] { $name }, çözümlenemeyen bir sütuna başvuruyor
    }
# Surface 12 (Invisible characters and broken objects). One whole sentence per kind ($kind selector); the name stays an isolated argument.
# Review: PO
hv-explorer-broken-body = Sütun son okumadan sonra silinmiş ya da yeniden adlandırılmış olabilir. Denetlemek için tabloyu yenileyin.
# Surface 13 (Settings › Metadata).
# Review: PO
hv-settings-nav-metadata = Üst veri
# Surface 13 (Settings › Metadata).
# Review: PO
hv-settings-section-metadata = ÜST VERİ
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-refresh = DDL sonrası gezgini yenile
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-always = Her zaman
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-ask = Sor
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-never = Hiçbir zaman
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-refresh-note = Bir konsol şemayı değiştiren bir ifade çalıştırdıktan sonra. Transaction içindeyse COMMIT'i bekler.
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-check = Değişiklikleri arka planda denetle
# Surface 13 (Settings › Metadata).
# Review: PO
hv-metadata-check-auto = Otomatik
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-check-note = Otomatik: yerel ve geliştirme bağlantıları 10 dakikada bir, diğerleri bağlandığınızda ya da Yenile'ye bastığınızda denetlenir.
# Surface 13 (Settings › Metadata).
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-metadata-every-min = { $n } dakikada bir
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-cache-conn = Bağlantı başına önbellek
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-cache-total = Tüm bağlantılar için önbellek
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-cache-note = Sınır aşılınca en uzun süre önce yüklenen şemalar önce çıkar. Geçerli veritabanı her zaman kalır.
# Surface 13 (Settings › Metadata).
# Review: PO
hv-metadata-unit-mb = MB
# Surface 13 (Settings › Metadata).
# Review: PO
hv-metadata-unit-gb = GB
# Surface 13 (Settings › Metadata). Settings › Metadata row.
# Review: PO
hv-metadata-set-clear = Önbellekteki üst veri
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Review: PO
hv-metadata-clear = Önbellekteki üst veriyi temizle…
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
#   $size (String) - a size (an hv-metadata-size-mb message), e.g. "38.4 MB".
# Review: PO
hv-metadata-clear-note = { $name } etkin konsolun bağlantısı. Önbellekteki üst verisi bu bilgisayarda { $size } kullanıyor.
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-metadata-clear-title = { $name } için önbellekteki üst veri temizlensin mi?
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Review: PO
hv-metadata-clear-body = Gezgin bunu bir sonraki bağlanışta sunucudan yeniden okur. Sunucuda hiçbir şey değişmez.
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Review: PO
hv-metadata-clear-btn = Temizle
# Surface 13 (Settings › Metadata). Settings › Metadata: clearing a connection's cached metadata.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-metadata-cleared = { $name } için önbellekteki üst veri temizlendi
# Surface 13 (Settings › Metadata).
# Variables:
#   $n (Number) - a count, e.g. 3400.
# Review: PO
hv-metadata-size-mb = { $n } MB
# Surface 14 (Long introspection). Progress of a pass in the explorer (surface 14: explorer only).
# Variables:
#   $fraction (Number) - the share read so far, from 0 to 1, e.g. 0.23.
# Review: PO
hv-explorer-progress-meta = { NUMBER($fraction, style: "percent") }
# Surface 14 (Long introspection). Progress of a pass in the explorer (surface 14: explorer only).
# Variables:
#   $schema (String) - a schema (MySQL: database) name, verbatim, e.g. "public".
#   $n (Number) - a count, e.g. 3400.
#   $total (Number) - the total, e.g. 15000.
# Review: PO
hv-explorer-progress-tip = { $schema } okunuyor: şu ana dek { $total } tablonun { $n } tanesinin sütunları. Bu sırada gezgin kullanılabilir.

# M4 command titles (palette, menus, keymap; `hv-cmd-<action id with . and _ as ->`).
# Review: PO
hv-cmd-explorer-view-ddl = DDL'i göster
# Review: PO
hv-cmd-explorer-copy-qualified-name = Tam adı kopyala
# Review: PO
hv-cmd-explorer-collapse-all = Gezginde tümünü kapat
# Review: PO
hv-cmd-explorer-open-data = Veriyi aç
# Review: PO
hv-cmd-explorer-refresh = Üst veriyi yenile
# Review: PO
hv-cmd-explorer-expand-all-children = Tüm alt öğeleri aç
# Review: PO
hv-cmd-explorer-edit-scope = Kapsamı düzenle
# Review: PO
hv-cmd-explorer-set-level-auto = Okuma düzeyi: Otomatik
# Review: PO
hv-cmd-explorer-set-level-names = Okuma düzeyi: Yalnız adlar
# Review: PO
hv-cmd-explorer-set-level-structure = Okuma düzeyi: Yapı
# Review: PO
hv-cmd-explorer-set-level-sources = Okuma düzeyi: Yapı ve kaynaklar
# Review: PO
hv-cmd-metadata-clear-cache = Önbellekteki üst veriyi temizle
# Review: PO
hv-cmd-ddl-view-copy = DDL'i kopyala

## m4-t14-explorer

# Review: PO
hv-explorer-not-loaded = Yüklenmedi
hv-explorer-count = { $n }
# Review: PO
hv-explorer-fk-target = → { $target } ({ $columns })
# Review: PO
hv-explorer-out-of-scope = kapsam dışı
# Review: PO
hv-explorer-filter-not-loaded = Yalnız yüklenmiş adlarda arar. { $n } bağlantı ya da şema henüz yüklenmedi.

## m4-t20-scope

# Review: PO
hv-scope-summary-pattern = Desen { $pattern }
# Review: PO
hv-scope-tree-mixed = kısmen seçili
# Review: PO
hv-scope-rewrite-title = Desen işaretli adlardan yeniden yazılsın mı?
# Review: PO
hv-scope-rewrite-body = Listede gösterilemeyen kısımları (bir düzenli ifade, bir dışlama, listede olmayan bir ad) kaldırılır.
# Review: PO
hv-scope-rewrite-keep = Deseni koru
# Review: PO
hv-scope-rewrite-apply = Yeniden yaz
# Review: PO
hv-scope-stored-unreadable = Kaydedilen şema kapsamı okunamadı. Gösterilen desen varsayılandır; dosya okunabilene kadar kaydedilemez.
# Review: PO
hv-scope-save-cache-failed = Bu bağlantının önbellekteki üst verisi silinemedi; hiçbir şey kaydedilmedi.
# Review: PO
hv-scope-save-data-unreadable = Bu bağlantının gezgin ayarları dosyası okunamıyor; hiçbir şey kaydedilmedi.
# Review: PO
hv-scope-save-data-failed = Şema kapsamı kaydedilemedi; hiçbir şey kaydedilmedi.
# Review: PO
hv-explorer-level-not-saved = Okuma düzeyi kaydedilemedi.

## m4-t24-live
# Review: PO
hv-cmd-explorer-copy-name = Adı kopyala
# Review: PO
hv-cmd-explorer-hide-like-this = Benzer nesneleri gizle
# Review: PO
hv-cmd-explorer-show-hidden = Gizli nesneleri göster
# Review: PO
hv-cmd-explorer-collapse-children = Tüm alt öğeleri kapat
# Review: PO
hv-cmd-explorer-level-menu = Okuma düzeyi
# Review: PO
hv-cmd-explorer-modify = Değiştir
# Review: PO
hv-explorer-open-data-other-db = Bu bağlantının konsolları { $current } veritabanını kullanıyor; bu yüzden Veriyi aç { $database } verisini gösteremiyor. Görmek için { $database } veritabanına bağlanın.
# Review: PO
hv-explorer-open-data-db-unknown = Hacivat bu bağlantının konsollarının hangi veritabanını kullandığını bilemiyor; bu yüzden Veriyi aç { $database } verisini açmadı. Bağlantının veritabanını ayarlayın ya da önce bir konsol açın.
# Review: PO
hv-explorer-hide-refused = { $name } benzeri nesneler gizlenemiyor: ad bir görünüm filtresi olarak yazılamıyor.

## m4-t25-ddl-view
# Surface 10 (DDL view). The toolbar's read-only chip (never hides).
# Review: PO
hv-ddl-view-read-only = salt okunur
# Surface 10 (DDL view). The definition could not be read (not loaded, or the login can't read it).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-not-loaded = Hacivat { $name } tanımını okuyamadı.
# Surface 10 (DDL view). Under hv-ddl-view-not-loaded.
# Review: PO
hv-ddl-view-not-loaded-body = Yenile, tanımı sunucudan yeniden okur.
# Surface 11 (Stale object). The "no longer exists" notice's button.
# Review: PO
hv-ddl-view-close-tab = Sekmeyi kapat
# Surface 10 (DDL view). The tooltip of "Definitions provided by server" on SQLite, where it is
# checked and can't be changed (ruling R-W4-12).
# Review: PO
hv-ddl-view-server-fixed-tip = SQLite her zaman sunucunun metnini gösterir: CHECK kısıtları ve üretilen sütunlar yalnızca orada bulunur.
# The palette title of the DDL view's Reload (F5 in its text).
# Review: PO
hv-cmd-ddl-view-reload = DDL'i yeniden yükle
# Surface 11 (Stale object). While the state check runs (review m-2): a quiet caption.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-ddl-view-checking = { $name } nesnesinin sunucuda değişip değişmediği denetleniyor…

## m4-t27-refresh-ui
# Surface 13 (Settings › Metadata). Clearing a connection's cached metadata failed (its files could not be deleted).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-metadata-clear-failed = { $name } için önbellekteki üst veri silinemedi. Hiçbir şey değişmedi.
# Surface 9 (Refresh after DDL). Ruling R-W5-10: the Ask when a whole schema changed (dynamic SQL, a database-wide change): no count.
# Variables:
#   $schema (String) - a schema (or database) name, verbatim, e.g. "public".
# Review: PO
hv-metadata-refresh-ask-schema = Gezgin, { $schema } içindeki değişiklikleri gösterecek şekilde yenilensin mi?
# Surface 9 (Refresh after DDL). Ruling R-W5-10: the namespace stamp's tooltip when the whole schema waits for COMMIT.
# Variables:
#   $schema (String) - a schema (or database) name, verbatim, e.g. "public".
# Review: PO
hv-metadata-after-commit-tip-schema = Bu transaction'ın { $schema } üzerindeki değişiklikleri COMMIT'i bekliyor. ROLLBACK bunları atar ve gezgin olduğu gibi kalır.
# Surface 9 (Refresh after DDL). Ruling R-W5-10: the output line when the whole schema waits for COMMIT.
# Variables:
#   $schema (String) - a schema (or database) name, verbatim, e.g. "public".
# Review: PO
hv-metadata-log-pending-schema = Gezgin yenilemesi COMMIT'i bekliyor ({ $schema } içindeki değişiklikler)

## m4-t32-shots
# Settings › General (PO ruling R-W5-14 a). The time zone row's title.
# Review: PO
hv-settings-time-zone = Saat dilimi
# Settings › General (PO ruling R-W5-14 a). The choice that follows the operating system, with the zone it reports.
# Variables:
#   $zone (String) - an IANA time zone, verbatim, e.g. "Europe/Istanbul".
# Review: PO
hv-settings-time-zone-system = Sistem ({ $zone })
# Settings › General (PO ruling R-W5-14 a). Coordinated Universal Time.
hv-settings-time-zone-utc = UTC
# Settings › General (PO ruling R-W5-14 a). The note under the time zone row.
# Review: PO
hv-settings-time-zone-note = Sonuçlardaki tarih ve saatler bu dilimde gösterilir. Bundan sonra açılan bağlantılar bunu kullanır; açık olanlar kendi dilimlerini korur.
# Command palette (PO ruling R-W5-14 b): clears the selected connection's query history.
# Review: PO
hv-cmd-connection-clear-history = Bağlantının sorgu geçmişini temizle
# The connection context menu (PO ruling R-W5-14 b).
# Review: PO
hv-conn-menu-clear-history = Sorgu geçmişini temizle…
# The query history dialog (PO ruling R-W5-14 b). The ⋯ button's name and tooltip.
# Review: PO
hv-history-more = Diğer işlemler
# The query history dialog's ⋯ menu (PO ruling R-W5-14 b).
# Review: PO
hv-history-clear-item = Bu bağlantının geçmişini temizle…
# Command palette (PO ruling R-W5-14 b): clears the history of the console's connection, from the history dialog.
# Review: PO
hv-cmd-history-clear = Bu bağlantının sorgu geçmişini temizle
# The clear-history confirmation (PO ruling R-W5-14 b). Cancel is the default button.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-history-clear-title = { $name } için sorgu geçmişi temizlensin mi?
# The clear-history confirmation (PO ruling R-W5-14 b).
# Review: PO
hv-history-clear-body = Bu bağlantı için kaydedilen tüm ifadeler, sabitlenenler de dahil, bu bilgisayardan silinir. Bu işlem geri alınamaz. Sunucuda hiçbir şey değişmez.
# The clear-history confirmation (PO ruling R-W5-14 b). The button that clears.
# Review: PO
hv-history-clear-btn = Geçmişi temizle
# A toast after the history of a connection was cleared (PO ruling R-W5-14 b).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-history-cleared = { $name } için sorgu geçmişi temizlendi.
# A toast when clearing the history of a connection failed (PO ruling R-W5-14 b).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-history-clear-failed = { $name } için sorgu geçmişi temizlenemedi. Hiçbir şey değişmedi.
# Settings › General (fix round 1, review M-3). A saved zone the time zone database does not know, as a choice.
# Variables:
#   $zone (String) - an IANA time zone, verbatim, e.g. "Europe/Istanbul".
# Review: PO
hv-settings-time-zone-unknown-choice = { $zone } (bilinmiyor)
# Settings › General (fix round 1, review M-3). The notice under the row while the saved zone is unknown.
# Variables:
#   $zone (String) - an IANA time zone, verbatim, e.g. "Europe/Istanbul".
#   $system (String) - the system choice as the select shows it, e.g. "System (Europe/Istanbul)".
# Review: PO
hv-settings-time-zone-unknown = { $zone } Hacivat'ın tanıdığı bir saat dilimi değil, bu yüzden { $system } kullanılıyor.

## m4-t33b-carries

# The clear-history confirmation when damaged backups exist (T32 carry, ruling 2026-11-20).
# Variables:
#   $count (Number) - how many damaged backups of the history exist, e.g. 2.
# Review: PO
hv-history-clear-body-backups = Bu bağlantı için kaydedilen tüm ifadeler, sabitlenenler de dahil, bu bilgisayardan silinir. Bu işlem geri alınamaz. Sunucuda hiçbir şey değişmez. Hacivat ayrıca geçmişin, dosya onarılırken bir kenara ayrılan { $count } hasarlı yedeğini saklıyor. Bunlar bu bağlantının ve diğer tüm bağlantıların ifadelerini hâlâ içerebilir.
# The clear-history confirmation: the checkbox (off by default) that also deletes the damaged backups.
# Review: PO
hv-history-clear-backups = Hasarlı geçmiş yedeklerini de sil
# The clear-history confirmation: the checkbox's dimmer note.
# Variables:
#   $count (Number) - how many damaged backups of the history exist, e.g. 2.
# Review: PO
hv-history-clear-backups-note = { $count } dosya, tüm bağlantıların eski ifadeleriyle
# A toast after the history of a connection was cleared and the damaged backups deleted.
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
#   $count (Number) - how many damaged backups were deleted, e.g. 2.
# Review: PO
hv-history-cleared-backups = { $name } için sorgu geçmişi temizlendi ve { $count } hasarlı yedek silindi.
# A toast when the history file turned out damaged while clearing with the history off (T32 re-review RR-2).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-history-clear-damaged = Geçmiş dosyası hasarlıydı, bu yüzden Hacivat onu yedek olarak bir kenara ayırdı. { $name } için sorgu geçmişi ondan temizlenmedi; hasarlı yedeği silmek için yeniden temizleyin.
# Surface 1 (Explorer tree). The Built-in group's tooltip when only some of its schemas come from the shipped catalog (T32 re-review RR-3).
# Variables:
#   $version (String) - the bundled catalog's server version, e.g. "17".
#   $schemas (String) - the schema names the bundled catalog answers for, a list, e.g. "information_schema".
# Review: PO
hv-explorer-builtin-tip-mixed = { $schemas } için yerleşik katalog (sürüm { $version }): Hacivat ile gelir, bağlantı olmadan okunur. Diğer yerleşik şemalar sunucudan okundu.
# Surface 4 (Filter field). Under the field when more objects match than the filter opens (T14 carry, R-W2-13).
# Variables:
#   $n (Number) - how many matches the filter did not open, e.g. 50.
# Review: PO
hv-explorer-filter-more = { $n } eşleşme daha açılmadı. Gruplarını açın ya da filtreyi daraltmak için yazmaya devam edin.
# Surface 14 (Long introspection). The meta of a loaded schema row while it is read again: the share read, in whole percents (T24 carry).
# Variables:
#   $fraction (Number) - the share read so far, from 0 to 1, e.g. 0.23.
hv-explorer-progress-percent = { NUMBER($fraction, style: "percent", minimumFractionDigits: 0, maximumFractionDigits: 0) }
# Surface 13 (Settings › Metadata). The clear-cache question's subtitle: the size of the cached metadata (T27 carry).
# Variables:
#   $size (String) - a preformatted size, e.g. "38.4 MB".
# Review: PO
hv-metadata-clear-subtitle = Bu bilgisayarda { $size }
# A toast when the history of a connection was cleared but its damaged backups could not be deleted (T33b review 6).
# Variables:
#   $name (String) - an object or connection name, verbatim, e.g. "müşteri".
# Review: PO
hv-history-cleared-backups-failed = { $name } için sorgu geçmişi temizlendi, ancak hasarlı yedekleri silinemedi.

## m4b-t01b-seams

# Surface 1 (Scripts node).
# Review: PO
hv-scripts-node = Betikler
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-node-shared = paylaşılan
# Surface 1 (Scripts node).
# Variables:
#   $conns (String) - the other connections, a locale list of names (data), e.g. "orders-staging and orders-dev".
# Review: PO
hv-scripts-node-shared-tip = Ayrıca şunların altında: { $conns }. Bu adda veritabanı olan her bağlantı aynı dosyaları gösterir.
# Surface 1 (Scripts node).
# Variables:
#   $path (String) - a folder or file path, verbatim (data), e.g. "~/work/orders-scripts".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-lib-tip = Klasör: { $path }. { $database } veritabanına bağlanır.
# Surface 1 (Scripts node).
# Variables:
#   $env (String) - the environment word (an env-* message), e.g. "production".
# Review: PO
hv-scripts-env-folder-tip = Bu klasörü yalnızca { $env } etiketli bağlantılar görür.
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-modified = M
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-modified-tip = Değiştirildi: son commit'ten farklı
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-new = N
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-new-tip = Yeni: son commit'te yok
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-conflicted = C
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-conflicted-tip = Çakışma: birleştirme çözülmedi
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-unknown = ?
# Surface 1 (Scripts node).
# Variables:
#   $reason (String) - selector: the reason; one of dataless, filter (anything else: the default).
# Review: PO
hv-scripts-git-unknown-tip =
    { $reason ->
        [dataless] Git durumu bilinmiyor: dosya indirilmedi; Hacivat karşılaştırmak için onu indirmez.
       *[filter] Git durumu bilinmiyor: dosyaya bir .gitattributes filtresi uygulanıyor; Hacivat filtreleri hiç çalıştırmaz.
    }
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-contains = •
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-git-contains-tip = Değişmiş dosyalar içeriyor
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-outside-link-tip = Kitaplığın dışına işaret ediyor; Hacivat bunu hiç açmaz.
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-dataless = indirilmedi
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-downloading = İndiriliyor…
# Surface 1 (Scripts node).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-diag-row = { $n } sorun
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-missing-row = Klasör bulunamadı
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-loading = Betikler listeleniyor…
# Surface 2 (Context menus).
# Review: PO
hv-explorer-menu-attach-library = Betik kitaplığı bağla…
# Surface 2 (Context menus).
# Review: PO
hv-explorer-menu-map-library = Bir betik kitaplığına eşle…
# Surface 2 (Context menus).
# Review: PO
hv-explorer-menu-map-library-none = Bağlı başka kitaplık yok
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-new-script = Yeni betik
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-new-folder = Yeni klasör
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-open = Aç
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-open-under = Başka bir bağlantıda aç…
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-rename = Yeniden adlandır
# Surface 2 (Context menus).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of other, windows (anything else: the default).
# Review: PO
hv-scripts-menu-trash =
    { $os ->
        [windows] Geri Dönüşüm Kutusu'na taşı
       *[other] Çöp Sepeti'ne taşı
    }
# Surface 2 (Context menus).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of linux, macos, windows (anything else: the default).
# Review: PO
hv-scripts-menu-reveal =
    { $os ->
        [windows] Dosya Gezgini'nde göster
        [linux] Bulunduğu klasörü aç
       *[macos] Finder'da göster
    }
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-copy-path = Yolu kopyala
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-copy-rel = Göreli yolu kopyala
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-settings = Betik kitaplığı ayarları…
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-outside-tip = Kitaplığın dışına işaret ediyor
# Surface 2 (Context menus).
# Review: PO
hv-scripts-new-name = adsız.sql
# Surface 2 (Context menus).
# Review: PO
hv-scripts-new-folder-name = yeni klasör
# Surface 2 (Context menus).
# Variables:
#   $reason (String) - selector: the reason; one of dots, empty, exists, ext, reserved, slash (anything else: the default).
#   $name (String) - a file or folder name, verbatim (data), e.g. "archive_old_orders.sql".
# Review: PO
hv-scripts-name-err =
    { $reason ->
        [slash] Ad / ya da \ içeremez.
        [reserved] Windows bu adı aygıtlar için ayırır (CON, PRN, AUX, NUL, COM1…).
        [exists] { $name } bu klasörde zaten var.
        [dots] Ad . ya da .. olamaz; nokta ya da boşlukla bitemez.
        [ext] Betik adı .sql ya da .md ile biter.
       *[empty] Bir ad yazın.
    }
# Surface 2 (Context menus).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of other, windows (anything else: the default).
#   $name (String) - a file or folder name, verbatim (data), e.g. "archive_old_orders.sql".
# Review: PO
hv-scripts-trashed =
    { $os ->
        [windows] { $name } Geri Dönüşüm Kutusu'na taşındı. Oradan geri yükleyebilirsiniz.
       *[other] { $name } Çöp Sepeti'ne taşındı. Oradan geri alabilirsiniz.
    }
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-choose = Betik klasörü seçin
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-title = Betik kitaplığı bağla
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-folder = Klasör
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-change = Değiştir…
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-database = Veritabanı
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-database-hint = Bu betiklerin ait olduğu veritabanı. Bu adda veritabanı olan her bağlantı onları gösterir.
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-aliases = Takma adlar
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-aliases-ph = Başka adları, virgülle ayırarak
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-shows-under = Görüneceği yerler
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-attach-shows-none = Henüz { $database } adlı veritabanı olan bağlantı yok.
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-will-write = Hacivat klasörde şu dosyayı oluşturur:
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-file-note = Yalnızca veritabanı adını ve takma adlarını içerir; bağlantı, sunucu ya da parola asla yazılmaz. Ekibiniz aynı bağlamayı alsın diye commit edin.
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-attach-found = .hacivat/workspace.toml bulundu — { $database } veritabanına bağlar
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-found-note = Hacivat bu dosyayı okur, hiç değiştirmez.
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-attach-btn = Bağla
# Surface 3 (Attach dialog).
# Variables:
#   $reason (String) - selector: the reason; one of already, control, empty, long, many (anything else: the default).
# Review: PO
hv-scripts-attach-err =
    { $reason ->
        [control] Veritabanı adı denetim karakteri içeremez.
        [long] Veritabanı adı en çok 128 bayt olabilir.
        [already] Bu klasör zaten bağlı.
        [many] Hacivat en çok 32 kitaplık tutar. Önce Ayarlar › Betik kitaplıkları'ndan birini kaldırın.
       *[empty] Veritabanının adını yazın.
    }
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-map-title = { $database } altında bir betik kitaplığı göster
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-map-library = Kitaplık
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-map-binds = { $database } veritabanına bağlı
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-map-note = Eşleme yalnızca bu bilgisayarda tutulur. Ekip arkadaşlarınız görmez.
# Surface 3 (Attach dialog).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-map-alias = { $database } adını, bu klasörü kullanan herkes için takma ad olarak da ekle
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-map-alias-sub = .hacivat/workspace.toml dosyasına ekler, dosyadaki yorumları korur
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-map-btn = Burada göster
# Surface 3 (Attach dialog).
# Review: PO
hv-scripts-map-other = Başka klasör bağla…
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-settings-nav-scripts = Betik kitaplıkları
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-settings-section-scripts = Betik kitaplıkları
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-attach = Klasör bağla…
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-note = Kitaplık, .sql ve .md dosyalarından oluşan bir klasördür. Kaldırmak klasöre ve dosyalarına hiç dokunmaz.
# Surface 4 (Settings › Script libraries).
# Variables:
#   $state (String) - selector: the watcher state; one of limit, polling, unavailable, watching (anything else: the default).
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-set-status =
    { $state ->
        [polling] 2 saniyede bir denetleniyor
        [unavailable] Klasör bulunamadı
        [limit] { $n } dosyada durdu
       *[watching] Değişiklikler izleniyor
    }
# Surface 4 (Settings › Script libraries).
# Variables:
#   $reason (String) - selector: the reason; one of network, watches (anything else: the default).
# Review: PO
hv-scripts-set-status-tip =
    { $reason ->
        [watches] Sistemin dosya izleme sınırı düşük; Hacivat değişiklik bildirimi beklemek yerine bu klasörü 2 saniyede bir denetler.
       *[network] Bu klasör bir ağ sürücüsünde; Hacivat değişiklik bildirimi beklemek yerine onu 2 saniyede bir denetler.
    }
# Surface 4 (Settings › Script libraries).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-set-files = { $n } betik
# Surface 4 (Settings › Script libraries).
# Variables:
#   $aliases (String) - the aliases, a locale list (data), e.g. "orders_p".
# Review: PO
hv-scripts-set-aliases = takma adlar: { $aliases }
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-git = git deposu
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-git-off = git durumu kapalı
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-remove = Kaldır…
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-mappings = Yerel eşlemeler
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-mapping-shows = gösterir:
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-mapping-remove = Kaldır
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-set-empty = Henüz betik kitaplığı yok. Betiklerini ait oldukları veritabanlarının altında görmek için .sql ve .md dosyalarından oluşan bir klasör bağlayın.
# Surface 4 (Settings › Script libraries).
# Variables:
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
# Review: PO
hv-scripts-remove-title = { $library } Hacivat'tan kaldırılsın mı?
# Surface 4 (Settings › Script libraries).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-remove-body = Klasör ve dosyaları, .hacivat/workspace.toml dahil, olduğu gibi kalır. Betikler düğümleri ve { $n } yerel eşlemesi kaldırılır.
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-remove-btn = Kaldır
# Surface 5 (Script tab).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
# Review: PO
hv-scripts-tab-title = { $file } · { $conn }
# Surface 5 (Script tab).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-tab-title-db = { $file } · { $conn } · { $database }
# Surface 5 (Script tab).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-tab-title-unbound = { $file } · bağlantı yok
# Surface 5 (Script tab).
# Variables:
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
# Review: PO
hv-scripts-tab-tip-library = { $library } kitaplığından
# Surface 5 (Script tab).
# Variables:
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-tab-tip-runs = { $conn } üzerinde, { $database } veritabanında çalışır
# Surface 5 (Script tab).
# Review: PO
hv-scripts-tab-tip-unbound = Hiçbir yerde çalışmaz: bağlantısı silindi
# Surface 5 (Script tab).
# Review: PO
hv-scripts-tab-tip-unsaved = Kaydedilmemiş değişiklikler
# Surface 5 (Script tab).
# Review: PO
hv-scripts-tab-save = Kaydet
# Surface 5 (Script tab).
# Review: PO
hv-scripts-tab-save-all = Tümünü kaydet
# Surface 5 (Script tab).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-tab-saved = { $file } kaydedildi
# Surface 5 (Script tab).
# Review: PO
hv-scripts-run-md = Markdown notları çalışmaz
# Surface 5 (Script tab).
# Review: PO
hv-scripts-run-unbound = Bu sekmenin bağlantısı silindi; hiçbir şey çalıştırmaz. Başka bir bağlantıda açın.
# Surface 5 (Script tab).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-run-switching = { $database } veritabanına geçiliyor…
# Surface 5 (Script tab).
# Review: PO
hv-scripts-unbound-title = Bu sekmenin açıldığı bağlantı silindi.
# Surface 5 (Script tab).
# Review: PO
hv-scripts-unbound-body = Metin korunuyor. Bir bağlantıda açana kadar hiçbir yerde çalışmaz.
# Surface 5 (Script tab).
# Variables:
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-use-log = Bu sekmenin oturumu artık { $database } veritabanını kullanıyor
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-disk-changed = { $file } diskte değişti.
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-changed-body = Bu sekmedeki düzenlemeleriniz kaydedilmedi. Yeniden yükle onları atar; Benimkini koru onları tutar ve kaydederken üzerine yazmadan önce sorar.
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-reload = Yeniden yükle
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-keep = Benimkini koru
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-disk-reload-title = Düzenlemeleriniz atılıp { $file } yeniden yüklensin mi?
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-reload-body = Diskteki dosya bu sekmedeki metnin yerini alır. Düzenlemeleriniz atılır.
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-reload-yes = At ve yeniden yükle
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-disk-reloaded = { $file } diskte değiştiği için yeniden yüklendi
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $new (String) - the new file name, verbatim (data), e.g. "haftalık_rapor.sql".
# Review: PO
hv-scripts-disk-renamed = { $file } dosyasının adı { $new } oldu; bu sekme onu izliyor
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-disk-deleted = { $file } diskten silindi.
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-deleted-body = Metin bu sekmede duruyor. Kaydet dosyayı yeniden yazar.
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-overwrite-title = Dosya siz açtıktan sonra diskte değişti. Üzerine yazılsın mı?
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-overwrite-body = Kaydetmek, Hacivat dışında yapılan değişiklikler dahil, diskteki sürümün yerini alır.
# Surface 6 (Changes on disk).
# Review: PO
hv-scripts-disk-overwrite = Üzerine yaz
# Surface 6 (Changes on disk).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
# Review: PO
hv-scripts-unlisted = { $file } artık { $conn } altında listelenmiyor.
# Surface 6 (Changes on disk).
# Variables:
#   $reason (String) - selector: the reason; one of binding, env (anything else: the default).
#   $folder (String) - a folder name, verbatim (data), e.g. "@prod".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
# Review: PO
hv-scripts-unlisted-body =
    { $reason ->
        [binding] Kitaplığı artık bu veritabanına bağlı değil. Bu sekme yine yalnızca { $conn } üzerinde çalışır.
       *[env] Başka bir ortamın klasörüne, { $folder } içine taşındı. Bu sekme yine yalnızca { $conn } üzerinde çalışır.
    }
# Surface 7 (Go to script).
# Review: PO
hv-cmd-scripts-goto = Betiğe git
# Surface 7 (Go to script).
# Review: PO
hv-cmd-file-save = Betiği kaydet
# Surface 7 (Go to script).
# Review: PO
hv-cmd-file-save-all = Tüm betikleri kaydet
# Surface 7 (Go to script).
# Review: PO
hv-cmd-scripts-attach = Betik kitaplığı bağla…
# Surface 7 (Go to script).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of linux, macos, windows (anything else: the default).
# Review: PO
hv-cmd-scripts-reveal =
    { $os ->
        [windows] Betiği Dosya Gezgini'nde göster
        [linux] Betiğin klasörünü aç
       *[macos] Betiği Finder'da göster
    }
# Surface 2 (Context menus). The palette title of F2 on a script file or folder row (controller ruling c).
# Review: PO
hv-cmd-scripts-rename = Betiği veya klasörü yeniden adlandır
# Surface 7 (Go to script).
# Review: PO
hv-scripts-goto-ph = Ada göre betiğe git
# Surface 7 (Go to script).
# Review: PO
hv-scripts-goto-none = Eşleşen betik yok
# Surface 7 (Go to script).
# Review: PO
hv-scripts-goto-empty = Henüz betik kitaplığı yok
# Surface 7 (Go to script).
# Review: PO
hv-scripts-goto-empty-body = Bir veritabanının menüsünden ya da Ayarlar › Betik kitaplıkları'ndan klasör bağlayın.
# Surface 7 (Go to script).
# Review: PO
hv-scripts-goto-hint-open = Aç
# Surface 7 (Go to script).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-choose-title = { $file } şu bağlantıda açılsın:
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-current = geçerli
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-prod-note = Üretim bağlantısı sizin yerinize hiç seçilmez.
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-none = Bu dosyayı gösteren bağlantı yok.
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-none-row = Bağlantı olmadan aç
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-none-body = Bağlantısız açılır; bir bağlantıda açana kadar hiçbir şey çalıştırmaz.
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-hint-choose = Seç
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-hint-back = Geri
# Surface 8 (Secret warning).
# Variables:
#   $kind (String) - selector: the finding's kind; one of key, password, token, url (anything else: the default).
# Review: PO
hv-scripts-secret-gutter =
    { $kind ->
        [token] Erişim belirtecine benziyor
        [key] Özel anahtara benziyor
        [url] Parola içeren bir bağlantı adresine benziyor
       *[password] Parolaya benziyor
    }
# Surface 8 (Secret warning).
# Review: PO
hv-scripts-secret-gutter-hint = Kaydederken önce sorulur. Bu satırdaki -- hacivat:allow-secret gibi bir yorum (tirelerden sonra bir boşlukla) onu susturur.
# Surface 8 (Secret warning).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-secret-title = { $file } içinde { $n } satır gizli bilgiye benziyor
# Surface 8 (Secret warning).
# Review: PO
hv-scripts-secret-body = Hacivat parolaya ya da belirtece benzeyen metin buldu. Değerler burada gösterilmez.
# Surface 8 (Secret warning).
# Variables:
#   $line (Number) - a line number, shown without grouping, e.g. 48213.
# Review: PO
hv-scripts-secret-line = Satır { NUMBER($line, useGrouping: "false") }
# Surface 8 (Secret warning).
# Review: PO
hv-scripts-secret-git = Bu dosyayı commit ederseniz değer, depoyu okuyabilen herkesle paylaşılır.
# Surface 8 (Secret warning).
# Variables:
#   $marker (String) - the literal hacivat:allow-secret (never translated).
# Review: PO
hv-scripts-secret-allow = Bir satırı bilerek tutmak için o satıra -- { $marker } (tirelerden sonra bir boşlukla) ya da /* { $marker } */ yorumunu ekleyin.
# Surface 8 (Secret warning).
# Review: PO
hv-scripts-secret-remember = Yine de kaydet, siz çıkana kadar bu sekme için hatırlanır.
# Surface 8 (Secret warning).
# Review: PO
hv-scripts-secret-save = Yine de kaydet
# Surface 9 (Large files).
# Review: PO
hv-scripts-run-as-script = Betik olarak çalıştır
# Surface 9 (Large files).
# Review: PO
hv-scripts-run-as-script-tip = Dosyanın tamamını diskten, her seferinde bir ifade olmak üzere ve her birini Çalıştır'ın denetimlerinden geçirerek gönderir. İlk hatada durur.
# Surface 9 (Large files).
# Review: PO
hv-scripts-run-running = Betik olarak çalışıyor
# Surface 9 (Large files).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-run-sent = { $n } ifade gönderildi
# Surface 9 (Large files).
# Variables:
#   $line (Number) - a line number, shown without grouping, e.g. 48213.
# Review: PO
hv-scripts-run-line = satır { NUMBER($line, useGrouping: "false") }
# Surface 9 (Large files).
# Variables:
#   $read (String) - bytes read so far (an hv-scripts-size-mb message), e.g. "38 MB".
#   $size (String) - a size (an hv-scripts-size-mb message), e.g. "148 MB".
# Review: PO
hv-scripts-run-read = { $read } / { $size }
# Surface 9 (Large files).
# Review: PO
hv-scripts-run-busy = Betik olarak çalışıyor: bitmesini bekleyin ya da durdurun
# Surface 9 (Large files).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-run-stopped = { $n }. ifadede durdu: sunucu hata döndürdü. Ondan sonrası gönderilmedi.
# Surface 9 (Large files).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-run-done = { $file } dosyasından { $n } ifade çalıştırıldı
# Surface 9 (Large files).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-run-cancelled = { $n } ifadeden sonra durduruldu. Sonrası gönderilmedi.
# Surface 9 (Large files).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-size-mb = { $n } MB
# Surface 10 (Encoding).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-enc-title = { $file } UTF-8 metin değil.
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-body = Kaydedildiği kodlamayı seçin ya da önce bakmak için salt okunur açın.
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-label = Kodlama
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-open = Aç
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-ro = Salt okunur aç
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-ro-chip = salt okunur
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-ro-note = UTF-8 olarak gösteriliyor; okunamayan karakterler � olarak görünür. Düzenlemek için bir kodlamayla yeniden açın.
# Surface 10 (Encoding).
# Variables:
#   $enc (String) - selector: the encoding id; one of iso-8859-9, other, utf-16le, windows-1252, windows-1254 (anything else: the default).
#   $encoding (String) - an encoding's name (data), e.g. "Windows-1254".
# Review: PO
hv-scripts-enc-name =
    { $enc ->
        [windows-1254] Windows-1254 (Türkçe)
        [iso-8859-9] ISO-8859-9 (Türkçe)
        [windows-1252] Windows-1252 (Batı)
        [utf-16le] UTF-16 LE
       *[other] { $encoding }
    }
# Surface 10 (Encoding).
# Variables:
#   $encoding (String) - an encoding's name (data), e.g. "Windows-1254".
# Review: PO
hv-scripts-enc-bom = { $encoding }, BOM'lu
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-seg-tip = Kodlama ve satır sonları. Kaydetmek ikisini de korur.
# Surface 10 (Encoding).
# Variables:
#   $encoding (String) - an encoding's name (data), e.g. "Windows-1254".
# Review: PO
hv-scripts-enc-menu-head = Kodlama: { $encoding }
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-reopen = Başka kodlamayla yeniden aç…
# Surface 10 (Encoding).
# Review: PO
hv-scripts-enc-save-with = Başka kodlamayla kaydet…
# Surface 10 (Encoding).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-enc-unencodable-title = { $file } kaydedilemedi
# Surface 10 (Encoding).
# Variables:
#   $line (Number) - a line number, shown without grouping, e.g. 48213.
#   $char (String) - the character that cannot be encoded (data), e.g. "漢".
#   $encoding (String) - an encoding's name (data), e.g. "Windows-1254".
# Review: PO
hv-scripts-enc-unencodable-body = { NUMBER($line, useGrouping: "false") }. satırdaki { $char } karakteri { $encoding } ile yazılamaz. Silin ya da Başka kodlamayla kaydet… seçeneğini kullanın.
# Surface 11 (Diagnostics).
# Variables:
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-diag-title = { $library } içinde { $n } sorun
# Surface 11 (Diagnostics).
# Variables:
#   $folder (String) - a folder name, verbatim (data), e.g. "@prod".
#   $suggestion (String) - the tag it probably meant, e.g. "@production".
# Review: PO
hv-scripts-diag-unknown-env = { $folder } bir ortam etiketi değil; altındaki dosyalar gizlendi ({ $suggestion } mı demek istediniz?)
# Surface 11 (Diagnostics).
# Variables:
#   $folder (String) - a folder name, verbatim (data), e.g. "@prod".
# Review: PO
hv-scripts-diag-unknown-env-nosug = { $folder } bir ortam etiketi değil; altındaki dosyalar gizlendi.
# Surface 11 (Diagnostics).
# Review: PO
hv-scripts-diag-env-help = Ortam klasörleri @local, @dev, @test, @staging ve @production'dır.
# Surface 11 (Diagnostics).
# Variables:
#   $path (String) - a folder or file path, verbatim (data), e.g. "~/work/orders-scripts".
# Review: PO
hv-scripts-diag-contradict = { $path } iki ortam adlandırıyor; altındaki dosyalar her yerde gizlendi.
# Surface 11 (Diagnostics).
# Variables:
#   $limit (String) - selector: which limit; one of depth, files (anything else: the default).
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-diag-limit =
    { $limit ->
        [depth] { $n } düzeyden derin klasörler listelenmez.
       *[files] Listeleme { $n } dosyada durdu; klasörün geri kalanı gösterilmiyor.
    }
# Surface 11 (Diagnostics).
# Variables:
#   $reason (String) - selector: the reason; one of network, watches (anything else: the default).
# Review: PO
hv-scripts-diag-polling =
    { $reason ->
        [watches] Sistemin dosya izleme sınırı düşük; değişiklikler 2 saniyede bir denetleniyor.
       *[network] Bu klasör bir ağ sürücüsünde; değişiklikler 2 saniyede bir denetleniyor.
    }
# Surface 11 (Diagnostics).
# Variables:
#   $reason (String) - selector: the reason; one of invalid, large, newer (anything else: the default).
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $folder (String) - a folder name, verbatim (data), e.g. "@prod".
# Review: PO
hv-scripts-diag-toml =
    { $reason ->
        [newer] { $file } daha yeni bir Hacivat ile yazılmış; kitaplık klasör adıyla, { $folder } olarak bağlanıyor.
        [large] { $file } 64 KB'tan büyük olduğu için okunmadı; kitaplık klasör adıyla, { $folder } olarak bağlanıyor.
       *[invalid] { $file } geçerli bir TOML değil; kitaplık klasör adıyla, { $folder } olarak bağlanıyor.
    }
# Surface 11 (Diagnostics).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-diag-toml-keys = { $file } Hacivat'ın kullanmadığı anahtarlar içeriyor. Bunlar yok sayılır.
# Surface 11 (Diagnostics).
# Review: PO
hv-scripts-diag-parser = Ayrıştırıcı mesajı
# Surface 11 (Diagnostics).
# Review: PO
hv-scripts-diag-untrusted = Git durumu kapalı: depo başka bir kullanıcıya ait.
# Surface 11 (Diagnostics).
# Review: PO
hv-scripts-diag-untrusted-help = Git de böyle bir depoyu güvenli olarak işaretlenene kadar (safe.directory) reddeder. Hacivat onun için rozet göstermez.
# Surface 11 (Diagnostics).
# Variables:
#   $path (String) - a folder or file path, verbatim (data), e.g. "~/work/orders-scripts".
# Review: PO
hv-scripts-diag-missing = Klasör yerinde yok: { $path }. Bağlı olmayan bir sürücüde olabilir.
# Surface 12 (Recovery).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-recover-title = { $n } betikteki kaydedilmemiş değişiklikler kurtarıldı
# Surface 12 (Recovery).
# Review: PO
hv-scripts-recover-body = Hacivat bunlar kaydedilmeden kapandı. Sekmeleri değişikliklerinizle yeniden açık; korumak için kaydedin.
# Surface 12 (Recovery).
# Review: PO
hv-scripts-recover-show = Göster
# Surface 12 (Recovery).
# Review: PO
hv-scripts-recover-dismiss = Kapat
# Surface 12 (Recovery).
# Variables:
#   $date (DateTime) - when the recovery copy was written, e.g. 2026-11-21 14:05.
# Review: PO
hv-scripts-recover-tab = { DATETIME($date, month: "short", day: "numeric", hour: "numeric", minute: "numeric") } tarihli kaydedilmemiş değişiklikler kurtarıldı.
# Surface 12 (Recovery).
# Review: PO
hv-scripts-recover-tab-body = Kaydet onları korur. At, diskteki dosyaya geri döner.
# Surface 12 (Recovery).
# Review: PO
hv-scripts-recover-discard = Değişiklikleri at…
# Surface 12 (Recovery).
# Variables:
#   $what (String) - selector: what is missing; one of file, library (anything else: the default).
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
# Review: PO
hv-scripts-recover-missing =
    { $what ->
        [library] { $library } kitaplığı artık bağlı değil.
       *[file] { $file } artık kitaplıkta yok.
    }
# Surface 12 (Recovery).
# Review: PO
hv-scripts-recover-missing-body = Kurtarılan metniniz bu sekmede. Kaydet onu aynı yere geri yazar.
# Surface 5 (Script tab).
# Variables:
#   $bound (String) - the database the tab is bound to (data), e.g. "kampanya".
#   $current (String) - the database the session is on (data), e.g. "api".
# Review: PO
hv-scripts-run-refused = Çalıştırma reddedildi: bu sekme { $bound } üzerinde çalışır, ama oturumu { $current } üzerinde.
# Surface 5 (Script tab).
# Variables:
#   $reason (String) - selector: the reason; one of edited, mysql (anything else: the default).
#   $bound (String) - the database the tab is bound to (data), e.g. "kampanya".
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
#   $current (String) - the database the session is on (data), e.g. "api".
# Review: PO
hv-scripts-run-refused-body =
    { $reason ->
        [edited] { $conn } artık { $current } veritabanını açıyor: bağlantı düzenlendi. Hiçbir şey gönderilmedi. Betiği { $bound } veritabanını gösteren bağlantıda açın.
       *[mysql] Hacivat oturumu bir kez geri geçirdi, yine de { $bound } üzerinde değil. Hiçbir şey gönderilmedi. Yeniden deneyin ya da betiği başka bir bağlantıda açın.
    }
# Surface 5 (Script tab).
# Variables:
#   $bound (String) - the database the tab is bound to (data), e.g. "kampanya".
# Review: PO
hv-scripts-run-refused-retry = { $bound } veritabanına geç ve çalıştır
# Surface 5 (Script tab).
# Review: PO
hv-scripts-run-refused-unbound = Çalıştırma reddedildi: bu sekmenin bağlantısı yok.
# Surface 6b (Unsaved changes).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-unsaved-title = { $file } dosyasındaki değişiklikler kaydedilsin mi?
# Surface 6b (Unsaved changes).
# Review: PO
hv-scripts-unsaved-body = Kaydetmezseniz değişiklikleriniz kaybolur.
# Surface 6b (Unsaved changes).
# Variables:
#   $total (Number) - the total, e.g. 2.
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-unsaved-count = { $total } kaydedilmemiş betikten { $n }. betik
# Surface 6b (Unsaved changes).
# Review: PO
hv-scripts-unsaved-save = Kaydet
# Surface 6b (Unsaved changes).
# Review: PO
hv-scripts-unsaved-discard = Kaydetme
# Surface 1 (Scripts node).
# Variables:
#   $file (String) - a script's file name, verbatim (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-downloading-tab = { $file } indiriliyor…
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-downloading-body = Bulutta tutuluyor; indirme bitince metin görünür. O zamana dek hiçbir şey çalışmaz.
# Surface 2 (Context menus).
# Review: PO
hv-scripts-menu-open-under-none = Bu dosyayı gösteren başka bağlantı yok
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-remove-body-none = Klasör ve dosyaları, .hacivat/workspace.toml dahil, olduğu gibi kalır. Betikler düğümleri kaldırılır.
# Surface 1 (Scripts node).
# Variables:
#   $n (Number) - a count, e.g. 3.
# Review: PO
hv-scripts-node-row-tip = Bu bağlantının altında { $n } betik listeleniyor.
# Surface 7 (Go to script).
# Review: PO
hv-scripts-choose-hint-close = Kapat
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-dataless-row-tip = Bulutta tutuluyor, indirilmedi. Açınca önce indirilir.
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-downloading-row-tip = İndiriliyor… İndirme bitince sekme metni gösterir.

## m4b-t13-explorer
# Surface 1 (Scripts node). What assistive technology reads for a file's git badge.
# Review: PO
hv-scripts-a11y-modified = değiştirildi
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-a11y-new = yeni
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-a11y-conflicted = çakışmalı
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-a11y-unknown = git durumu bilinmiyor
# Surface 1 (Scripts node). A folder that holds changed files.
# Review: PO
hv-scripts-a11y-contains = değişiklik içeriyor
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-a11y-outside = kitaplığın dışını gösteriyor
# Surface 1 (Scripts node).
# Review: PO
hv-scripts-a11y-dataless = indirilmedi
# Surface 1 (Scripts node). The "?" badge's text alternative for the reasons hv-scripts-git-unknown-tip does not cover.
# Variables:
#   $reason (String) - selector: the reason; one of case, notafile, submodule, unsafepath (anything else: the default).
# Review: PO
hv-scripts-git-unknown-other-tip =
    { $reason ->
        [submodule] Git durumu bilinmiyor: bu bir alt modül ve Hacivat alt modül açmaz.
        [case] Git durumu bilinmiyor: depo bu adı başka büyük/küçük harflerle yazıyor.
        [unsafepath] Git durumu bilinmiyor: depo bu dosyayı Hacivat'ın izlemediği bir adla tanımlıyor.
       *[notafile] Git durumu bilinmiyor: normal bir dosya değil.
    }
# Surface 1 (Scripts node). The Scripts node's child row when the library folder can't be listed.
# Variables:
#   $reason (String) - selector: why; one of moved, responding, unreadable (anything else: the default).
# Review: PO
hv-scripts-unavailable-row =
    { $reason ->
        [responding] Klasör yanıt vermiyor
        [moved] Klasör taşındı ya da değiştirildi
       *[unreadable] Klasör okunamıyor
    }
# Surface 11 (Diagnostics). The problems the other diagnostics messages do not cover.
# Variables:
#   $what (String) - selector: the problem; one of badname, hidden, linkscope, manifest, moved, noname, rootenv, suppressed, unreadable (anything else: the default).
#   $n (Number) - a count, e.g. 3.
#   $folder (String) - the library's folder name (data), e.g. "orders-scripts".
# Review: PO
hv-scripts-diag-other =
    { $what ->
        [badname] Bazı adlar adlandırma kurallarına uymuyor; o dosyalar listelenmiyor.
        [unreadable] Bir klasör okunamıyor; içindeki dosyalar listelenmiyor.
        [linkscope] Bir bağlantı başka bir ortamın klasörünü gösteriyor; hiçbir yerde listelenmiyor.
        [suppressed] { $n } sorun daha listelenmiyor.
        [hidden] { $n } sorun daha bu bağlantının göstermediği dosyalarla ilgili.
        [noname] Ne .hacivat/workspace.toml ne de { $folder } klasör adı bir veritabanı adı; kitaplık yalnızca eşlendiği yerde görünür.
        [rootenv] Kitaplık klasörü bir ortam klasörünün içinde; içindeki hiçbir şey listelenmiyor.
        [moved] Kitaplık klasörü taşındı ya da değiştirildi; içindeki hiçbir şey listelenmiyor.
       *[manifest] .hacivat/workspace.toml kullanılamıyor; kitaplık klasör adına, { $folder }, göre bağlanıyor.
    }
# Surface 2 (Context menus). The reasons hv-scripts-name-err does not cover, under the name field.
# Variables:
#   $reason (String) - selector: the reason; one of chars, hidden, long (anything else: the default).
# Review: PO
hv-scripts-name-err-more =
    { $reason ->
        [long] Bir ad en çok 255 bayt olabilir.
        [hidden] Bir ad noktayla başlayamaz; node_modules listelenmez.
       *[chars] Bir ad < > : " | ? * ya da denetim karakterleri içeremez.
    }
# Surface 2 (Context menus). The toast's title when a file operation was refused.
# Variables:
#   $op (String) - selector: the operation; one of create, rename, trash (anything else: the default).
#   $os (String) - selector: the host (macos, windows, linux); one of linux, macos, windows (anything else: the default).
#   $name (String) - the file or folder name (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-op-failed-title =
    { $op ->
        [create] { $name } oluşturulamadı
        [rename] { $name } yeniden adlandırılamadı
       *[trash]
            { $os ->
                [windows] { $name } Geri Dönüşüm Kutusu'na taşınamadı
               *[other] { $name } Çöp Sepeti'ne taşınamadı
            }
    }
# Surface 2 (Context menus). The toast's body: why, in plain words; nothing was changed.
# Variables:
#   $reason (String) - selector: the reason; one of changed, exists, gone, hidden, io, large, listed, name, nohardlinks, path, scope, trash, unavailable (anything else: the default).
# Review: PO
hv-scripts-op-failed =
    { $reason ->
        [exists] Bu klasörde aynı adı taşıyan bir şey zaten var. Hiçbir şey değişmedi.
        [scope] Bu, dosyaları hangi bağlantıların gördüğünü değiştirirdi. Hiçbir şey değişmedi; istiyorsanız dosya yöneticinizde yeniden adlandırın.
        [large] Bu klasör denetlenemeyecek kadar büyük; dosya yöneticinizde kaldırın.
        [nohardlinks] Bu sürücü, aynı adlı bir dosyayı tehlikeye atmadan yeniden adlandıramıyor. Hiçbir şey değişmedi; dosya yöneticinizde yeniden adlandırın.
        [changed] Çöp kutusuna taşındı ama yoldaki bir klasör bu arada değişti. Çöp kutusunu denetleyin.
        [gone] Artık kitaplıkta değil. Hiçbir şey değişmedi.
        [listed] Bu bağlantı o klasörü göstermiyor; Hacivat onu buradan değiştirmez.
        [hidden] Klasörde bu bağlantının göstermediği dosyalar var; Hacivat onları buradan taşımaz.
        [name] Bu ad burada kullanılamaz. Hiçbir şey değişmedi.
        [path] Yoldaki bir klasör değişti ya da bağlantıya dönüştü; Hacivat durdu. Hiçbir şey değişmedi.
        [unavailable] Kitaplık klasörü şu anda kullanılamıyor. Hiçbir şey değişmedi.
        [trash] Sistemin çöp kutusu kabul etmedi. Dosya yerinde duruyor.
       *[io] Sistem izin vermedi. Hiçbir şey değişmedi.
    }
# Surface 2 (Context menus). Palette title.
# Review: PO
hv-cmd-scripts-open = Betiği aç
# Surface 2 (Context menus). Palette title.
# Review: PO
hv-cmd-scripts-new-script = Yeni betik
# Surface 2 (Context menus). Palette title.
# Review: PO
hv-cmd-scripts-new-folder = Yeni betik klasörü
# Surface 2 (Context menus). Palette title; no default chord (controller ruling c).
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of linux, macos, windows (anything else: the default).
# Review: PO
hv-cmd-scripts-trash =
    { $os ->
        [windows] Betiği Geri Dönüşüm Kutusu'na taşı
       *[other] Betiği Çöp Sepeti'ne taşı
    }
# Surface 2 (Context menus). Palette title.
# Review: PO
hv-cmd-scripts-copy-path = Betik yolunu kopyala
# Surface 2 (Context menus). Palette title.
# Review: PO
hv-cmd-scripts-copy-relative-path = Betiğin göreli yolunu kopyala
# Surface 2 (Context menus). Palette title.
# Review: PO
hv-cmd-scripts-refresh = Betik kitaplığını yenile
# Surface 2 (Context menus). Under the name field when the name holds invisible or bidi characters, shown escaped.
# Variables:
#   $name (String) - the name with invisible characters written as <U+XXXX> (data), e.g. "a<U+202E>lqs.sql".
# Review: PO
hv-scripts-name-hidden-chars = Bu ad gizli karakterler içeriyor: { $name }
# Surface 2 (Context menus). Moved to Trash, but a folder on the way changed meanwhile.
# Variables:
#   $os (String) - selector: the host (macos, windows, linux); one of linux, macos, windows (anything else: the default).
#   $name (String) - the file or folder name (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-trashed-changed =
    { $os ->
        [windows] { $name } Geri Dönüşüm Kutusu'na taşındı ama yoldaki bir klasör bu arada değişti. Geri Dönüşüm Kutusu'nu denetleyin.
       *[other] { $name } Çöp Sepeti'ne taşındı ama yoldaki bir klasör bu arada değişti. Çöp Sepeti'ni denetleyin.
    }
## m4b-t14-tabs

# Surface 5 (Script tab): the tab while its file is read.
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-opening = { $file } açılıyor…
# Surface 5 (Script tab): the file could not be read; the tab stays and runs nothing.
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-open-failed = { $file } açılamadı.
# Surface 5 (Script tab).
# Variables:
#   $reason (String) - selector: why; one of missing, library, scope, binary, refused, lossy, encoding (anything else: the default).
#   $conn (String) - the connection's name (data), e.g. "orders-staging".
# Review: PO
hv-scripts-open-failed-body =
    { $reason ->
        [missing] Artık kitaplıkta değil.
        [library] Kitaplığı bağlı değil ya da şu an erişilemiyor.
        [scope] { $conn } altında listelenmiyor.
        [binary] Metin değil, ikili bir dosyaya benziyor.
        [refused] Yolu bir betiğin taşıyabileceği bir yol değil ya da kitaplığın dışına çıkıyor.
        [lossy] Geri kaydetmek düzenlenmemiş baytları değiştirirdi, bu yüzden açılmadı.
        [encoding] Seçtiğiniz kodlamayla çözülemiyor.
       *[other] Dosya okunamadı.
    }
# Surface 5 (Script tab): a PostgreSQL or SQLite script is opened only on the database its session is on (R-M4b-25).
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
#   $conn (String) - the connection's name (data), e.g. "orders-prod".
#   $database (String) - the database the script was opened under (data), e.g. "orders_archive".
#   $current (String) - the database the connection runs on (data), e.g. "orders".
# Review: PO
hv-scripts-open-other-db = { $file } açılmadı: { $conn } { $database } üzerinde değil, { $current } üzerinde çalışıyor.
# Surface 5 (Script tab).
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
#   $conn (String) - the connection's name (data), e.g. "orders-prod".
#   $database (String) - the database the script was opened under (data), e.g. "orders".
# Review: PO
hv-scripts-open-db-unknown = { $file } açılmadı: { -brand-hacivat } { $conn } bağlantısının hangi veritabanında çalıştığını bilemiyor, bu yüzden { $database } olduğundan emin olamıyor.
# Surface 5 (Script tab): a MySQL profile set to one session cannot give a script tab its own session on another database.
# Variables:
#   $file (String) - the file name (data), e.g. "kampanya_özet.sql".
#   $conn (String) - the connection's name (data), e.g. "api-dev".
#   $database (String) - the database (data), e.g. "kampanya".
# Review: PO
hv-scripts-open-single-session = { $file } açılmadı: { $conn } her konsol için tek oturum kullanıyor, bu yüzden bir betik onun yanında { $database } üzerinde çalışamaz.
# Surface 5 (Script tab): the connection the tab was opened under no longer exists.
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-open-no-profile = { $file } açılmadı: bağlantısı artık yok.
# Surface 5 (Script tab): the session's database could not be read before Run (R-M4b-27).
# Variables:
#   $bound (String) - the database the tab is bound to (data), e.g. "orders".
# Review: PO
hv-scripts-run-refused-unknown = Çalıştırma reddedildi: bu sekme { $bound } üzerinde çalışır, ama { -brand-hacivat } oturumunun hangi veritabanında olduğunu bilemiyor.
# Surface 5 (Script tab).
# Review: PO
hv-scripts-run-refused-unknown-body = Hiçbir şey gönderilmedi. Yeniden bağlanın ya da betiği başka bir bağlantı altında açın.
# Surface 5 (Script tab): the MySQL switch made when the tab opened failed.
# Variables:
#   $database (String) - the database (data), e.g. "kampanya".
# Review: PO
hv-scripts-switch-failed = Bu sekmenin oturumu { $database } veritabanına geçemedi.
# Surface 5 (Script tab).
# Review: PO
hv-scripts-switch-failed-body = Yanlış veritabanında hiçbir şey çalışmasın diye Çalıştır kapalı kalır. Yeniden denemek için sekmeyi kapatıp betiği tekrar açın.
# Surface 9 (Large files): the status bar's segment of a T2 preview tab.
# Review: PO
hv-scripts-preview-segment = Salt okunur önizleme
# Surface 9 (Large files): why Save does nothing on a preview.
# Review: PO
hv-scripts-preview-no-save = Salt okunur önizleme kaydedilmez: dosyanın yalnızca başını tutar.
# Surface 9 (Large files): why Run does nothing on a preview.
# Review: PO
hv-scripts-preview-no-run = Salt okunur önizleme çalışmaz. Betik olarak çalıştır dosyanın tamamını gönderir.
# Surface 6 (Changes on disk): a save failed; the tab keeps its edits.
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-save-failed = { $file } kaydedilemedi
# Surface 6 (Changes on disk).
# Variables:
#   $reason (String) - selector: why; one of path, scope, damaged, missing (anything else: the default).
#   $conn (String) - the connection's name (data), e.g. "orders-staging".
# Review: PO
hv-scripts-save-failed-body =
    { $reason ->
        [path] Yolundaki bir klasör değişti ya da dosya bir bağlantıya dönüştü, bu yüzden hiçbir şey yazılmadı. Düzenlemeleriniz bu sekmede duruyor.
        [scope] Artık { $conn } altında listelenmiyor, bu yüzden hiçbir şey yazılmadı. Düzenlemeleriniz bu sekmede duruyor.
        [damaged] Dosya yazılamadı ve eski hâline getirilemedi. Metniniz bu sekmede ve bir kurtarma kopyasında duruyor.
        [missing] Kitaplığı bağlı değil ya da şu an erişilemiyor, bu yüzden hiçbir şey yazılmadı. Düzenlemeleriniz bu sekmede duruyor.
       *[other] Hiçbir şey yazılmadı. Düzenlemeleriniz bu sekmede duruyor.
    }
# Surface 6 (Changes on disk): the save went through, but a folder check after it failed.
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-saved-path-changed = { $file } kaydedildi, ama kaydedilirken yolundaki bir klasör değişti. Beklediğiniz yerde olduğunu denetleyin.
# Surface 6 (Changes on disk): a rename on disk the tab follows (carries-w1 T14).
# Variables:
#   $path (String) - the new path in the library (data), e.g. "reports/haftalık_rapor.sql".
# Review: PO
hv-scripts-moved = { $path } konumuna taşındı
# Surface 6 (Changes on disk).
# Variables:
#   $conn (String) - the connection's name (data), e.g. "orders-staging".
# Review: PO
hv-scripts-moved-body = Dosya diskte taşındı ve bu sekme onu izliyor. Yine yalnızca { $conn } üzerinde çalışır.
# Surface 6 (Changes on disk): the Deleted on disk notice's second button.
# Review: PO
hv-scripts-disk-close = Sekmeyi kapat
# Surface 12 (Recovery): a recovered copy whose writer may still be running (R-T5-5).
# Review: PO
hv-scripts-recover-unknown = Bu kopya hâlâ açık olan başka bir { -brand-hacivat } penceresine ait olabilir. Kaydedene ya da atana kadar saklanır.
# Surface 12 (Recovery): the launch notice's list line.
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
#   $conn (String) - the connection's name (data), e.g. "orders-prod".
# Review: PO
hv-scripts-recover-item = { $file } · { $conn }
# Surface 12 (Recovery).
# Variables:
#   $file (String) - the file name (data), e.g. "orders_report.sql".
# Review: PO
hv-scripts-recover-item-unbound = { $file } · bağlantı yok
# Surface 10 (Encoding): an encoding the user can open the file in (one button each).
# Variables:
#   $encoding (String) - the encoding's name (data), e.g. "windows-1254".
# Review: PO
hv-scripts-enc-open-as = { $encoding } olarak aç
# Fix round 1 (T14 review).
# Surface 5 (Script tab): a MySQL/MariaDB script tab never shares a session (R-T14-3); the profile cannot give it one of its own.
# Review: PO
hv-scripts-one-session = Bu bağlantı tek oturuma izin veriyor; konsolu kapatın ya da sınırı yükseltin.
# Surface 6 (Changes on disk): why a "No longer listed" tab is off (R-T14-4); the file moved where this connection does not show it.
# Review: PO
hv-scripts-unlisted-gone = Bu bağlantının göstermediği bir yere taşındı.
# Surface 6 (Changes on disk).
# Variables:
#   $folder (String) - the environment folder (data), e.g. "@production".
# Review: PO
hv-scripts-unlisted-env = Başka bir ortamın klasörü olan { $folder } içinde.
# Surface 6 (Changes on disk): Run and Save are off on a tab whose file is no longer listed (R-T14-4).
# Review: PO
hv-scripts-unlisted-off = Bu sekmede Çalıştır ve Kaydet kapalı; dosya artık ait olmadığı yerde hiç çalışmaz ya da yazılmaz.
# Surface 5 (Script tab): why "Open under another connection…" is disabled (a sentence, N2).
# Review: PO
hv-scripts-open-under-none-reason = Bu dosyayı gösteren başka bağlantı yok.
# Surface 5 (Script tab): why "Open under another connection…" is disabled until the chooser can pick (production is never chosen for you).
# Review: PO
hv-scripts-open-under-chooser = İstediğiniz bağlantının Betikler düğümünden açın.
# Surface 6b (Unsaved changes): the prompt's subtitle (I5).
# Variables:
#   $env (String) - the environment word, e.g. "PRODUCTION".
#   $conn (String) - the connection's name (data), e.g. "orders-prod".
#   $database (String) - the database (data), e.g. "orders".
# Review: PO
hv-scripts-unsaved-subtitle = { $env } · { $conn } · { $database }
# Surface 6b (Unsaved changes): the subtitle of a tab with no connection.
# Review: PO
hv-scripts-unsaved-subtitle-unbound = Bağlantı yok
# m4b-t14-tabs fix round 3 (R-T14-5): the first Run after a launch restore follows a SQLite
# file that is now on another volume (same path and inode).
# Variables:
#   $path (String) - the database file's path (data), e.g. "/Volumes/data/orders.db".
# Review: PO
hv-scripts-sqlite-remounted = Veritabanı dosyası artık başka bir birimde; { $path } üzerinde çalıştırılıyor.

## m4b-t16-attach

# The command behind the database menu's "Map to a script library…" (menu and palette).
# Review: PO
hv-cmd-scripts-map = Bir betik kitaplığına eşle…
# The command behind "Script library settings…" (Settings › Script libraries).
# Review: PO
hv-cmd-scripts-library-settings = Betik kitaplığı ayarları…
# Surface 3 (Attach dialog): why the folder can't be attached; nothing was written.
# Variables:
#   $reason (String) - selector: why; one of envfolder, link, home, app, root, nested, notfolder, readonly, changed (anything else: the default).
#   $folder (String) - an environment folder's name, verbatim (data), e.g. "@production".
# Review: PO
hv-scripts-attach-refused =
    { $reason ->
        [envfolder] Bu klasör { $folder } ortam klasörünün içinde. Bunun yerine { $folder } klasörünü içeren klasörü bağlayın.
        [link] Bu bir bağlantı. Bunun yerine işaret ettiği klasörü seçin.
        [home] Ana klasörünüz kitaplık olamaz. İçindeki bir betik klasörünü seçin.
        [app] Bu Hacivat'ın kendi klasörlerinden biri ya da birini içeriyor. Başka bir klasör seçin.
        [root] Bütün bir sürücü kitaplık olamaz. Bir betik klasörü seçin.
        [nested] Bu klasör bağlı bir kitaplığın içinde ya da bir kitaplığı içeriyor.
        [notfolder] Bu bir klasör değil.
        [readonly] Hacivat kitaplık listesini şu an değiştiremiyor. Nedeni Ayarlar › Betik kitaplıkları'nda.
        [changed] Hacivat dosyayı yazarken klasör değişti. Hiçbir şey bağlanmadı; yeniden deneyin.
       *[other] Hacivat bu klasörü bağlayamadı. Hiçbir şey yazılmadı.
    }
# Surface 3 (Attach dialog): an alias Hacivat can't write.
# Variables:
#   $reason (String) - selector: why; one of control, long, many (anything else: the default).
# Review: PO
hv-scripts-attach-aliases-err =
    { $reason ->
        [control] Takma ad denetim karakteri içeremez.
        [long] Takma ad en çok 128 bayt olabilir.
       *[many] Hacivat en çok 32 takma ad tutar.
    }
# Surface 3 (Attach dialog): the file found in the folder could not be read.
# Review: PO
hv-scripts-attach-found-unreadable = Hacivat bu dosyayı okuyamıyor, bu yüzden klasör kendi adıyla bağlanır. Hacivat dosyayı hiç değiştirmez.
# Surface 3 (Attach dialog): while the chosen folder is read.
# Review: PO
hv-scripts-attach-reading = Klasör okunuyor…
# Surface 3 (Attach dialog): the subtitle, the connection and database it was opened from.
# Variables:
#   $conn (String) - a connection name, verbatim (data), e.g. "orders-prod".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-attach-from = { $conn } › { $database }
# Surface 3 (Mapping): a library in the Library select.
# Variables:
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
#   $database (String) - a database name, verbatim (data), e.g. "orders".
# Review: PO
hv-scripts-map-option = { $library }, { $database } veritabanına bağlı
# Surface 3 (Mapping): why the share box can't be ticked.
# Variables:
#   $reason (String) - selector: why; one of none, unreadable (anything else: the default).
# Review: PO
hv-scripts-map-alias-off =
    { $reason ->
        [none] Bu klasörde ekleme yapılacak bir .hacivat/workspace.toml yok.
        [unreadable] Hacivat bu klasörün .hacivat/workspace.toml dosyasını okuyamıyor, bu yüzden ekleme yapamaz.
       *[name] Bu ad takma ad olamaz.
    }
# Surface 3 (Mapping): why nothing was saved.
# Variables:
#   $reason (String) - selector: why; one of changed, unavailable (anything else: the default).
# Review: PO
hv-scripts-map-failed =
    { $reason ->
        [changed] .hacivat/workspace.toml bu pencere onu okuduktan sonra değişti. Hiçbir şey kaydedilmedi; yeniden bakın.
        [unavailable] Kitaplığın klasörüne şu an erişilemiyor. Hiçbir şey kaydedilmedi.
       *[other] Hacivat eşlemeyi kaydedemedi. Hiçbir şey değişmedi.
    }
# Surface 4 (Settings › Script libraries): how a library's changes are found (R-T9-1).
# Review: PO
hv-scripts-set-watch = İzleme
# Surface 4 (Settings › Script libraries): the watcher's own choice.
# Review: PO
hv-scripts-set-watch-auto = Otomatik
# Surface 4 (Settings › Script libraries): checking the folder every 2 seconds.
# Review: PO
hv-scripts-set-watch-poll = 2 saniyede bir
# Surface 4 (Settings › Script libraries): why a library is polled, when the user chose it.
# Review: PO
hv-scripts-set-watch-chosen = Bu klasörün 2 saniyede bir denetlenmesini siz seçtiniz.
# Surface 4 (Settings › Script libraries): states beyond the mock's four.
# Variables:
#   $state (String) - selector: the state; one of moved, envfolder, unreadable, notresponding, unwatched (anything else: the default).
# Review: PO
hv-scripts-set-state =
    { $state ->
        [moved] Bu, bağlanan klasör değil: taşınmış ya da değiştirilmiş.
        [envfolder] Bu klasör bir ortam klasörünün içinde, bu yüzden içindekiler gösterilmiyor.
        [unreadable] Hacivat bu klasörü okuyamıyor.
        [notresponding] Klasör yanıt vermiyor.
        [unwatched] Bu klasördeki değişiklikleri bildiren yok; Yenile onları gösterir.
       *[scanning] Klasör okunuyor…
    }
# Surface 4 (Settings › Script libraries): the confirmed fix for a moved or replaced folder.
# Review: PO
hv-scripts-set-adopt = Bu klasörü kullan…
# Surface 4 (Settings › Script libraries).
# Variables:
#   $library (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
# Review: PO
hv-scripts-adopt-title = Bu klasör { $library } olarak kullanılsın mı?
# Surface 4 (Settings › Script libraries).
# Variables:
#   $path (String) - a folder path, verbatim (data), e.g. "~/work/orders-scripts".
# Review: PO
hv-scripts-adopt-body = Hacivat bu kitaplığı göstermeyi bıraktı, çünkü { $path } konumundaki klasör bağlanan klasör değil. Onu kendiniz taşıdıysanız ya da geri yüklediyseniz kullanın: kitaplık yerel eşlemelerini korur. Neden değiştiğini bilmiyorsanız önce klasöre bakın.
# Surface 4 (Settings › Script libraries).
# Review: PO
hv-scripts-adopt-btn = Bu klasörü kullan
# Surface 4 (Settings › Script libraries): the Remove and Use this folder questions' subtitle.
# Variables:
#   $path (String) - a folder path, verbatim (data), e.g. "~/work/orders-scripts".
hv-scripts-remove-path = { $path }
# Surface 11 (Diagnostics): nothing watches the library (R-T9-1: the poll suggestion).
# Variables:
#   $folder (String) - a library's folder name, verbatim (data), e.g. "orders-scripts".
# Review: PO
hv-scripts-diag-unwatched = { $folder } içindeki değişiklikleri bildiren yok. Hacivat dışında yapılan düzenlemeler görünmüyorsa Ayarlar › Betik kitaplıkları'nda İzleme: 2 saniyede bir'i seçin.
