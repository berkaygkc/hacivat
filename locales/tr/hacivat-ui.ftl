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
hv-conn-fix-require = Doğrulamadan şifrele
# Review: PO
hv-conn-fix-require-note = Bağlantı şifrelenir, ama { -brand-hacivat } sunucunun gerçekten iddia ettiği sunucu olduğunu denetlemez.
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
hv-conn-issue-tls-key-encrypted = Özel anahtar şifreli. Şifresiz bir anahtar dosyası kullanın.
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
