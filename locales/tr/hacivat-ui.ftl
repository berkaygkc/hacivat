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
hv-explorer-env = { $env ->
        [production] üretim
        [staging] hazırlık
        [test] test
        [dev] geliştirme
        [local] yerel
       *[unset] ortam yok
    }
# Review: PO
hv-explorer-write-failed = Değişiklik bağlantılar dosyanıza kaydedilemedi.
# Review: PO
hv-explorer-secret-failed = Kayıtlı bir parola sistemin anahtar deposunda güncellenemedi.
# Review: PO
hv-explorer-copied = URL olarak kopyalandı
# Review: PO
hv-explorer-copied-nopw = Parola dahil değil.
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
hv-capsule-step = { $step ->
        [pgpass] pgpass
        [resolve] Çözümle
        [connect] Bağlan
        [tls] TLS
        [sign-in] Oturum aç
        [check-server] Sunucuyu denetle
       *[open-file] Dosyayı aç
    }
# Review: PO
hv-capsule-unsupported = desteklenmiyor
# Review: PO
hv-capsule-access-read-write = okuma-yazma
# Review: PO
hv-capsule-latency = { $ms } ms
# Review: PO
hv-status-env = { $env ->
        [production] üretim
        [staging] hazırlık
        [test] test
        [dev] geliştirme
        [local] yerel
       *[unset] ortam yok
    } · { $access ->
        [read-only] salt okunur
       *[read-write] okuma-yazma
    }
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
hv-conn-import-env = { $env ->
        [production] Üretim
        [staging] Hazırlık
        [test] Test
        [dev] Geliştirme
        [local] Yerel
       *[unset] Seçin…
    }
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
