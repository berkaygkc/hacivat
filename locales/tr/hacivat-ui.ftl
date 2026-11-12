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
hv-conn-dlg-key-pass = Anahtar parolası
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
hv-conn-issue-tls-key-encrypted = Özel anahtar şifreli. Şifresiz bir anahtar dosyası kullanın.
# Review: PO
hv-conn-issue-tls-mode-unsupported = { $mode } bu motor için henüz kullanılamıyor.
# Review: PO
hv-conn-issue-option-rejected = { $key } özelliğine burada izin verilmiyor.
