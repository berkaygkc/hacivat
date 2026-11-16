### Core texts shared with other binaries: database and error explanations.
### Append-only: each task adds a block headed `## <task slug>`.

## m2a-t10-errors

# Connect errors (HV-CONN-03). The five mock texts (auth.invalid, network.dns, tls.untrusted,
# config.unknown_database, server.unsupported_version) are NOTES.md's Turkish column; the rest
# are new. Every entry waits for the PO's review.

# Review: PO
error-conn-network-dns-title = { $host } bulunamadı.
# Review: PO
error-conn-network-dns-body = Bu Mac sunucu adını çözümleyemedi. Yazımı denetleyin; sunucu özel bir ağdaysa VPN'e bağlanın.

# Review: PO
error-conn-network-refused-title = { $host } sunucusu { $port } numaralı bağlantı noktasında bağlantıyı reddetti.
# Review: PO
error-conn-network-refused-body = Sunucuya ulaşılabiliyor, ama bu bağlantı noktasında bağlantıyı kabul eden bir şey yok. Bağlantı noktasını ve veritabanı sunucusunun çalışıp ağ bağlantılarını kabul ettiğini denetleyin.

# Review: PO
error-conn-network-timeout-title = { $host } zamanında yanıt vermedi.
# Review: PO
error-conn-network-timeout-body = Bağlantı denemesi zaman aşımına uğradı. Sunucu meşgul olabilir ya da bir güvenlik duvarı bağlantıyı düşürüyor olabilir. Yeniden deneyin ya da sunucu adını ve bağlantı noktasını denetleyin.

# Review: PO
error-conn-network-reset-title = { $host } bağlantıyı kapattı.
# Review: PO
error-conn-network-reset-body = Bağlantı, Hacivat oturum açamadan kesildi. Sunucu yeniden başlamış ya da bir güvenlik duvarı veya vekil sunucu bağlantıyı kapatmış olabilir. Yeniden deneyin.

# Review: PO
error-conn-network-unreachable-title = { $host } sunucusuna bu Mac'ten ulaşılamıyor.
# Review: PO
error-conn-network-unreachable-body = Sunucuya giden bir ağ yolu yok. Ağ bağlantınızı denetleyin; sunucu özel bir ağdaysa VPN'e bağlanın.

# Review: PO
error-conn-network-ipv6-unavailable-title = { $host } sunucusuna yalnızca IPv6 ile ulaşılabiliyor.
# Review: PO
error-conn-network-ipv6-unavailable-body = Sunucunun yalnızca IPv6 adresleri var ve bu ağ IPv6 taşımıyor. Başka bir ağ deneyin ya da varsa sunucunun IPv4 adresini kullanın.

# Review: PO
error-conn-tls-handshake-title = { $host } ile güvenli bağlantı kurulamadı.
# Review: PO
error-conn-tls-handshake-body = Hacivat ile sunucu şifreleme konusunda anlaşamadı. Bağlantının TLS ayarlarını denetleyin; nedeni teknik ayrıntılarda.

# Review: PO
error-conn-tls-untrusted-title = { $host }, bu Mac'in güvenmediği bir sertifika gönderdi.
# Review: PO
# m2b-t14-pipeline (named exception): back to the approved mock text (`err-tls-b`).
error-conn-tls-untrusted-body = Kendinden imzalı ya da şirketinizin kendi CA'sınca verilmiş olabilir. Güvenmeden önce inceleyin.

# Review: PO
error-conn-tls-hostname-title = { $host } sunucusunun sertifikası başka bir ad için verilmiş.
# Review: PO
error-conn-tls-hostname-body = Sunucunun sertifikası, Hacivat'ın bağlandığı adı kapsamıyor. Sertifikada yazan bir adla bağlanın; örneğin IP adresi ya da takma ad yerine sunucunun tam alan adıyla.

# Review: PO
error-conn-tls-protocol-title = { $host } ile güvenli bağlantı bir güvenlik denetiminden geçemedi.
# Review: PO
error-conn-tls-protocol-body = Sunucu, Hacivat'ın kabul etmediği bir TLS sürümü kullanıyor ya da doğrulanamayan şifreli veri gönderdi. Hacivat TLS 1.2 veya sonrasını gerektirir. Sunucu güncelse ağdaki bir şey bağlantıya müdahale ediyor olabilir.

# Review: PO
error-conn-tls-client-cert-required-title = { $host } bir istemci sertifikası istiyor.
# Review: PO
error-conn-tls-client-cert-required-body = Sunucu yalnızca kendi sertifikasını sunan bağlantıları kabul ediyor. İstemci sertifikasını ve anahtarını bağlantının TLS ayarlarına ekleyin.

# Review: PO
error-conn-tls-not-supported-by-server-title = { $host } şifreli bağlantı sunmuyor.
# Review: PO
error-conn-tls-not-supported-by-server-body = Bu bağlantı TLS gerektiriyor ama sunucu desteklemiyor. Sunucuda TLS'i açın ya da ağ özel ve güvenilirse bağlantının TLS kipini değiştirin.

# Review: PO
error-conn-auth-invalid-title = Sunucu { $user } kullanıcısının parolasını reddetti.
# Review: PO
error-conn-auth-invalid-body = Parola değişmiş ya da süresi dolmuş olabilir. Hesap kilitlenmesin diye Hacivat 3 denemeden sonra durur.

# Review: PO
error-conn-auth-expired-title = { $user } kullanıcısının parolasının süresi doldu.
# Review: PO
error-conn-auth-expired-body = Sunucu, bu hesap oturum açmadan önce yeni bir parola istiyor. Parolayı başka bir araçla değiştirin ya da veritabanı yöneticinize başvurun, sonra yeni parolayı buraya girin.

# Review: PO
error-conn-auth-locked-title = { $user } hesabı kilitli.
# Review: PO
error-conn-auth-locked-body = Sunucu bu hesabın oturum açmasına izin vermiyor; genellikle çok sayıda başarısız denemeden sonra ya da bir yönetici kilitlediği için. Kilidi açması için veritabanı yöneticinize başvurun.

# Review: PO
error-conn-auth-method-unsupported-title = Sunucu, { $user } kullanıcısından Hacivat'ın kullanamadığı bir yöntemle oturum açmasını istiyor.
# Review: PO
error-conn-auth-method-unsupported-body = Hesap, Hacivat'ın bu bağlantı üzerinden desteklemediği bir kimlik doğrulama yöntemi kullanıyor. Bazı yöntemler yalnızca şifreli bağlantıda çalışır: TLS'i açmayı deneyin ya da hesabın hangi yöntemi kullandığını veritabanı yöneticinize sorun.

# Review: PO
error-conn-authz-database-title = { $user } kullanıcısının { $db } veritabanını açma izni yok.
# Review: PO
error-conn-authz-database-body = Sunucu bu hesabın buradan bu veritabanına bağlanmasına izin vermiyor. Erişim için veritabanı yöneticinize başvurun ya da hesabın açabildiği bir veritabanı seçin.

# Review: PO
error-conn-config-invalid-title = { $engine ->
        [SQLite] { $file } bir SQLite veritabanı değil.
       *[other] { $name } bağlantısının ayarları kullanılamıyor.
    }
# Review: PO
error-conn-config-invalid-body = { $engine ->
        [SQLite] Dosya var ama bir veritabanına benzemiyor. Doğru dosyayı seçtiğinizi denetleyin; başka türde bir dosya ya da bozuk olabilir.
       *[other] Bağlantıdaki bir ayar olduğu gibi kullanılamıyor. Hangisi olduğu teknik ayrıntılarda yazıyor; düzeltmek için bağlantıyı düzenleyin.
    }

# Review: PO
error-conn-config-unknown-database-title = { $engine ->
        [SQLite] { $file } açılamadı.
        [MySQL] { $host } üzerinde { $db } adlı bir veritabanı yok ya da { $user } kullanıcısı ona erişemiyor.
        [MariaDB] { $host } üzerinde { $db } adlı bir veritabanı yok ya da { $user } kullanıcısı ona erişemiyor.
       *[other] { $host } üzerinde { $db } adlı bir veritabanı yok.
    }
# Review: PO
error-conn-config-unknown-database-body = { $engine ->
        [SQLite] Dosya taşınmış ya da silinmiş olabilir veya bu Mac bulunduğu klasörü okuyamıyor. Yolu denetleyin ya da dosyayı yeniden seçin.
       *[other] Oturum açıldı. { $user } kullanıcısının erişebildiği veritabanları:
    }

# Review: PO
error-conn-server-too-many-connections-title = { $host } sunucusunda boş bağlantı kalmadı.
# Review: PO
error-conn-server-too-many-connections-body = Sunucu bağlantı sınırına ulaştı. Biraz sonra yeniden deneyin ya da artık gerekmeyen bağlantıları kapatın.

# Review: PO
error-conn-server-starting-title = { $host } hâlâ başlatılıyor.
# Review: PO
error-conn-server-starting-body = Veritabanı sunucusu çalışıyor ama henüz bağlantılara hazır değil. Biraz sonra yeniden deneyin.

# Review: PO
error-conn-server-shutdown-title = { $host } kapanıyor.
# Review: PO
error-conn-server-shutdown-body = Veritabanı sunucusu duruyor ya da bir yönetici oturumu sonlandırdı. Sunucu yeniden açılınca tekrar bağlanın.

# Review: PO
error-conn-server-read-only-title = { $engine ->
        [SQLite] { $file } yalnızca okunabiliyor.
       *[other] { $host } yalnızca okumaya izin veriyor.
    }
# Review: PO
error-conn-server-read-only-body = { $engine ->
        [SQLite] Hacivat bu dosyaya ya da bulunduğu klasöre yazamıyor. Dosyanın izinlerini denetleyin ya da yazabileceğiniz bir kopyasını açın.
       *[other] Sunucu salt okunur kipte; örneğin bir kopya (replica) olduğu için. Değişiklik yapmak için birincil sunucuya bağlanın.
    }

# Review: PO
error-conn-server-unsupported-version-title = { $host }, Hacivat'ın desteklemediği { $engine } { $version } sürümünü çalıştırıyor.
# Review: PO
error-conn-server-unsupported-version-body = { $engine ->
        [MySQL] Hacivat MySQL 8.0 ve sonrasını destekler.
        [MariaDB] Hacivat MariaDB 10.6 ve sonrasını destekler.
       *[other] Hacivat PostgreSQL 14 ve sonrasını destekler.
    } Şema okuma ve güvenlik denetimleri daha yeni sistem kataloglarına dayanır.

# Review: PO
error-conn-cancelled-title = { $name } bağlantısı iptal edildi.
# Review: PO
error-conn-cancelled-body = Hiçbir şey değişmedi. Hazır olduğunuzda yeniden bağlanın.

# Review: PO
error-conn-internal-title = { $name } bağlantısı kurulurken bir sorun oluştu.
# Review: PO
error-conn-internal-body = Hacivat beklemediği bir hatayla karşılaştı. Sunucunun mesajı teknik ayrıntılarda; sorunu bildirirken bunları kopyalayın.

## m2b-t14-pipeline

# Connect errors that M2b reaches (Task 2 NOTES, Turkish column; ssh.host_key_unknown is new).
# Every entry waits for the PO's review.

# Review: PO
error-conn-tls-pin-changed-title = { $host }, sabitlenenden farklı bir sertifika gönderdi.
# Review: PO
error-conn-tls-pin-changed-body = Hacivat oturum açmadan önce durdu. Sabitlemeyi değiştirmeden önce yeni parmak izini sunucuyu yöneten kişiyle karşılaştırın.

# Review: PO
error-conn-ssh-host-key-unknown-title = Hacivat { $hop_host } sunucusunun anahtarını henüz tanımıyor.
# Review: PO
error-conn-ssh-host-key-unknown-body = Bu Mac bu SSH sunucusuna daha önce bağlanmadı ve bağlantı anahtarına güvenip güvenmeyeceğinizi sormadı. Anahtarı incelemek için yeniden deneyin ya da bağlantının sunucu anahtarı ayarını denetleyin.

# Review: PO
error-conn-ssh-host-key-changed-title = { $hop_host } sunucusunun anahtarı değişti.
# Review: PO
error-conn-ssh-host-key-changed-body = Hacivat oturum açmadan önce durdu. Güvenmeden önce yeni anahtarı inceleyin.

# Review: PO
error-conn-ssh-auth-title = { $hop_host }, SSH oturumunu reddetti.
# Review: PO
error-conn-ssh-auth-body = Hacivat bu bağlantının izin verdiği oturum açma yöntemlerini denedi. Açık anahtarınızın o sunucudaki ~/.ssh/authorized_keys dosyasında olduğunu denetleyin ya da başka bir oturum açma yöntemi seçin.

# Review: PO
error-conn-ssh-channel-open-title = { $hop_host } tünellere izin vermiyor (AllowTcpForwarding)
# Review: PO
error-conn-ssh-channel-open-body = { $hop_host } oturumu açıldı, ancak sunucu bağlantıyı { $host } adresine iletmeyi reddetti. Yöneticisinden TCP iletimine, PermitOpen ayarlıysa { $host } adresine de izin vermesini isteyin.

# Review: PO
error-conn-ssh-agent-unavailable-title = Hacivat ssh-agent'a ulaşamıyor.
# Review: PO
error-conn-ssh-agent-unavailable-body = Bu bağlantı { $hop_host } oturumunu ssh-agent'taki anahtarlarla açar, ancak Hacivat için SSH_AUTH_SOCK ayarlı değil. Ajanı başlatıp anahtarınızı ssh-add ile ekleyin ya da bağlantının SSH ayarlarında bir anahtar dosyası seçin.

# Review: PO
error-conn-ssh-host-key-revoked-title = { $hop_host }, iptal edilmiş olarak işaretli bir anahtar gönderdi.
# Review: PO
error-conn-ssh-host-key-revoked-body = Hacivat iptal edilmiş bir anahtarla asla bağlanmaz. Sunucunun güncel anahtarını yöneticisinden isteyin.

# Review: PO
error-conn-auth-insecure-method-title = Sunucu parolayı bu bağlantıda güvenli olmayan bir biçimde istedi.
# Review: PO
error-conn-auth-insecure-method-body-tls = Bağlantı şifreli, ancak sertifika doğrulanmadığı için Hacivat parolanın { $host } sunucusuna ulaşacağından emin olamıyor ve parolayı göndermedi. Sertifikayı inceleyip sabitleyin, sonra yeniden bağlanın.
# Review: PO
error-conn-auth-insecure-method-body-plain = Bağlantı şifreli değil; parola ağda başkalarının okuyabileceği ya da yeniden kullanabileceği bir biçimde taşınırdı. Hacivat parolayı göndermedi. Sunucuda TLS'i açın ya da bu ağda riski kabul ediyorsanız Şifreleme'yi Kapalı yapın.
# Review: PO
error-conn-auth-insecure-method-body = { error-conn-auth-insecure-method-body-plain }

## m3a-t15-sqlite-typed

# Review: PO
error-exec-sqlite-busy-hint = Bu veritabanı dosyası başka bir program tarafından kilitlenmiş. O program dosyayla işini bitirince deyimi yeniden çalıştırın.

## m3a-t23-exec

# Review: PO
exec-dur-lt-ms = <1 ms
# Review: PO
exec-dur-ms = { $ms } ms
# Review: PO
exec-dur-s = { $s } sn
# Review: PO
exec-dur-s-ms = { $s } sn { $ms } ms
# Review: PO
exec-dur-min = { $min } dk
# Review: PO
exec-dur-min-s = { $min } dk { $s } sn
# Review: PO
exec-size-kb = { $n } KB
# Review: PO
exec-size-mb = { $n } MB
# Review: PO
exec-size-gb = { $n } GB
# Review: PO
exec-rows-retrieved = { $rows } satır alındı ({ $duration })
# Review: PO
exec-rows-retrieved-more = { $rows } satır alındı ({ $duration }); devamı var
# Review: PO
exec-rows-affected = { $rows } satır etkilendi ({ $duration })
# Review: PO
exec-completed = Tamamlandı ({ $duration })
# Review: PO
exec-tx-committed = İşlem commit edildi
# Review: PO
exec-tx-rolled-back = İşlem geri alındı
# Review: PO
exec-summary = { $total } ifadeden { $executed } tanesi çalıştı, { $failed } tanesi başarısız oldu ({ $duration })
# Review: PO
exec-error = { $n }. ifade başarısız oldu
# Review: PO
exec-cancelled = { $duration } sonra iptal edildi
# Review: PO
exec-more-result-sets = { $skipped } sonuç kümesi daha okundu ve gösterilmedi; yalnızca ilki tutulur.
# Review: PO
exec-deny-read-only-input = Gönderilmedi: bu konsol salt okunur ve ifade veriyi ya da şemayı değiştirebilir.
# Review: PO
exec-deny-read-only-transaction = Gönderilmedi: salt okunur bir konsol işlem başlatamaz ya da bitiremez.
# Review: PO
exec-deny-read-only-protocol = Durduruldu: sunucu, salt okunur işlemin içinde bir yazma bildirdi.
# Review: PO
exec-deny-read-only-engine = Sunucu, işlem salt okunur olduğu için ifadeyi reddetti.
# Review: PO
exec-deny-read-only-function = Gönderilmedi: ifade, etkileri geri alınamayan bir fonksiyon çağırıyor.
# Review: PO
exec-events-dropped = { $n } olay gösterilmedi
# Review: PO
exec-copy-unsupported = Gönderilmedi: COPY … FROM STDIN ve TO STDOUT, istemcinin veri aktarmasını gerektirir; konsollar bunu yapmaz.
# Review: PO
exec-not-started = Hiçbir şey çalışmadı.
# Review: PO
exec-run-as-one-unsupported = Gönderilmedi: Hacivat bir seçimi ifade ifade çalıştırır; tek bir ifade olarak gönderemez.
# Review: PO
cursor-closed-session-needed = { $rows } satırdan sonra imleç kapandı: oturum başka bir ifade için gerekiyordu.
# Review: PO
cursor-closed-idle-timeout = { $rows } satırdan sonra imleç kapandı: { $duration } boyunca boşta kaldı.
# Review: PO
cursor-closed-tab-closed = { $rows } satırdan sonra imleç kapandı: sekmesi kapatıldı.
# Review: PO
cursor-closed-tx-ended = { $rows } satırdan sonra imleç kapandı: ait olduğu işlem sona erdi.
# Review: PO
cursor-closed-user-closed = { $rows } satırdan sonra imleç kapatıldı.
# Review: PO
cursor-closed-memory-limit = { $rows } satırdan sonra imleç kapandı: bellek sınırına ulaşıldı.
# Review: PO
cursor-closed-connection-lost = { $rows } satırdan sonra imleç kapandı: bağlantı koptu.
# Review: PO
cursor-closed-server-terminated = { $rows } satırdan sonra imleç kapandı: sunucu sorguyu bir hatayla durdurdu.
# Review: PO
cursor-closed-cancelled = { $rows } satırdan sonra imleç kapandı: getirme durduruldu.
# Review: PO
cursor-closed-enforcement-gap = { $rows } satırdan sonra imleç kapandı: salt okunur koruma oturumu kapattı.
# Review: PO
cursor-closed-rolled-back = Hacivat'ın işlemi geri alındı.
# Review: PO
cursor-closed-session-needed-rolled-back = { $rows } satırdan sonra imleç kapandı: oturum başka bir ifade için gerekiyordu. { cursor-closed-rolled-back }
# Review: PO
cursor-closed-idle-timeout-rolled-back = { $rows } satırdan sonra imleç kapandı: { $duration } boyunca boşta kaldı. { cursor-closed-rolled-back }
# Review: PO
cursor-closed-tab-closed-rolled-back = { $rows } satırdan sonra imleç kapandı: sekmesi kapatıldı. { cursor-closed-rolled-back }
# Review: PO
cursor-closed-tx-ended-rolled-back = { $rows } satırdan sonra imleç kapandı: ait olduğu işlem sona erdi. { cursor-closed-rolled-back }
# Review: PO
cursor-closed-user-closed-rolled-back = { $rows } satırdan sonra imleç kapatıldı. { cursor-closed-rolled-back }
# Review: PO
cursor-closed-memory-limit-rolled-back = { $rows } satırdan sonra imleç kapandı: bellek sınırına ulaşıldı. { cursor-closed-rolled-back }
# Review: PO
cursor-closed-connection-lost-rolled-back = { $rows } satırdan sonra imleç kapandı: bağlantı koptu. { cursor-closed-rolled-back }
# Review: PO
cursor-closed-server-terminated-rolled-back = { $rows } satırdan sonra imleç kapandı: sunucu sorguyu bir hatayla durdurdu. { cursor-closed-rolled-back }
# Review: PO
cursor-closed-cancelled-rolled-back = { $rows } satırdan sonra imleç kapandı: getirme durduruldu. { cursor-closed-rolled-back }
# Review: PO
cursor-closed-enforcement-gap-rolled-back = { $rows } satırdan sonra imleç kapandı: salt okunur koruma oturumu kapattı. { cursor-closed-rolled-back }
# Review: PO
exec-error-server = Sunucu ifadeyi reddetti.
# Review: PO
exec-error-connection-lost = Bağlantı koptu.
# Review: PO
exec-error-connection-lost-unknown = İfade çalışırken bağlantı koptu. Etkili olmuş da olabilir, olmamış da.
# Review: PO
exec-error-cancelled = İfade iptal edildi.
# Review: PO
exec-error-tx-failed = Gönderilmedi: işlem başarısız oldu; sunucu artık yalnızca geri almayı kabul eder.
# Review: PO
exec-error-tx-in-progress = Gönderilmedi: zaten açık bir işlem var.
# Review: PO
exec-error-tx-unsupported = Gönderilmedi: bu bağlantı o işlem komutunu çalıştıramaz.
# Review: PO
exec-error-enforcement-gap = Salt okunur koruma hiçbir şeyin yazılmadığını doğrulayamadı; bu yüzden oturumu kapattı.
# Review: PO
exec-error-not-connected = Gönderilmedi: konsol bağlı değil.
# Review: PO
exec-error-internal = Hacivat ifadeyi bir iç hata yüzünden çalıştıramadı.
# Review: PO
exec-uncertain-exec-comment = Yürütülebilir bir yorumun (/*! … */) içinde ; var.
# Review: PO
exec-uncertain-unterminated = Yukarıdaki bir tırnak, yorum ya da parantez kapanmamış.
# Review: PO
exec-uncertain-unknown-block = İfadenin BEGIN … END blokları birbirini tutmuyor.
# Review: PO
exec-uncertain-delimiter = İfadenin içindeki bir ;, geçerli DELIMITER ile ifadeyi ikiye böler.
# Review: PO
exec-uncertain-missing-semicolon = İki ifade arasında bir ; eksik görünüyor.
# Review: PO
exec-uncertain-comment-end = Yürütülebilir bir yorumun ya da optimizer ipucunun nerede bittiği sunucuya bağlı.
# Review: PO
exec-uncertain-client-command = İfadenin içindeki bir istemci komutu yazıldığı gibi çalıştırılamaz.
# Review: PO
exec-uncertain-overlapping-units = İmleçlerden ya da seçimlerden ikisi bir ifadenin bir kısmını paylaşıyor; ifade iki kez çalışırdı.

## m3b-t04-safety

# Review: PO
safety-class-read = Veri okur
# Review: PO
safety-class-session = Oturumu değiştirir
# Review: PO
safety-class-write = Veriyi değiştirir
# Review: PO
safety-class-ddl = Şemayı değiştirir
# Review: PO
safety-class-admin = Yönetimsel ya da dış yan etkileri var
# Review: PO
safety-class-unknown = Etkileri belirlenemiyor

# Review: PO
safety-reason-not-implemented = Hacivat bu ifadeyi henüz denetleyemiyor.
# Review: PO
safety-reason-lex-error = İfadede Hacivat'ın okuyamadığı bir tırnak, yorum ya da karakter var.
# Review: PO
safety-reason-client-command = Metinde sunucunun çalıştıramayacağı bir istemci komutu var.
# Review: PO
safety-reason-template-variable = Metinde doldurulmamış bir şablon değişkeni var.
# Review: PO
safety-reason-no-statement = Çalıştırılacak bir ifade yok.
# Review: PO
safety-reason-multi-statement = Metinde birden fazla ifade var.
# Review: PO
safety-reason-ambiguous-boundary = İfadenin nerede bittiği belirsiz.
# Review: PO
safety-reason-parse-error = İfade ayrıştırılamadı.
# Review: PO
safety-reason-parser-disagreement = Hacivat'ın ifadeyi iki okuyuşu birbirini tutmuyor.
# Review: PO
safety-reason-too-complex = İfade denetlenemeyecek kadar büyük ya da iç içe.
# Review: PO
safety-reason-syntax-newer-than-agent = İfade, Hacivat'ın bu sürümünün bilmediği daha yeni bir söz dizimi kullanıyor.
# Review: PO
safety-reason-unsupported-dialect = Hacivat bu veritabanının ifadelerini henüz denetleyemiyor.
# Review: PO
safety-reason-fn-denylisted = { $name }, yan etkisi olan işlevler listesinde.
# Review: PO
safety-reason-fn-unknown = Hacivat { $name } işlevinin ne yaptığını bilmiyor.
# Review: PO
safety-reason-operator-unknown = Hacivat { $name } işlecinin ne yaptığını bilmiyor.
# Review: PO
safety-reason-cast-unknown = Hacivat { $name } türüne dönüştürmenin ne yaptığını bilmiyor.
# Review: PO
safety-reason-executable-comment = Yürütülebilir bir yorum (/*! … */), yorumun içine gizlenmiş kodu çalıştırır.
# Review: PO
safety-reason-lex-mode-change = İfade, sunucunun SQL metnini okuma biçimini değiştiriyor.
# Review: PO
safety-reason-privilege-change = İfade rolleri ya da yetkileri değiştiriyor.
# Review: PO
safety-reason-data-modifying-cte = Bir WITH bölümü veriyi değiştiriyor.
# Review: PO
safety-reason-locking-clause = İfade satırları kilitliyor.
# Review: PO
safety-reason-explain-analyze = EXPLAIN ANALYZE ifadeyi çalıştırır.
# Review: PO
safety-reason-dynamic-sql = İfade, metinden oluşturulan SQL'i çalıştırıyor.
# Review: PO
safety-reason-procedural-code = İfade yordamsal kod çalıştırıyor.
# Review: PO
safety-reason-txn-control = İfade işlemi denetliyor.
# Review: PO
safety-reason-session-state = İfade oturumun ayarlarını değiştiriyor.
# Review: PO
safety-reason-external-file = İfade sunucudaki dosyaları okuyor ya da yazıyor.
# Review: PO
safety-reason-external-network = İfade ağ üzerinden başka bir sunucuya erişiyor.
# Review: PO
safety-reason-process-exec = İfade sunucuda bir program çalıştırıyor.
# Review: PO
safety-reason-admin-control = İfade sunucuyu yönetiyor.
# Review: PO
safety-reason-xid-consume = İfade bir işlem kimliği (XID) tüketiyor.
# Review: PO
safety-reason-sequence-advance = İfade bir diziyi (sequence) ilerletiyor.
# Review: PO
safety-reason-write-temp = İfade geçici bir tabloya yazıyor.
# Review: PO
safety-reason-hint-exec-time = Bir optimizer ipucu süre sınırını kaldırıyor.
# Review: PO
safety-reason-hint-set-var = Bir optimizer ipucu bir ayarı değiştiriyor.
# Review: PO
safety-reason-routine-call = İfade, etkileri denetlenemeyen bir yordamı çağırıyor.
# Review: PO
safety-reason-set-not-allowlisted = Salt okunur bağlantıda yalnızca zararsız ayarlar değiştirilebilir; bu onlardan biri değil.
# Review: PO
safety-reason-write-data = İfade veriyi değiştiriyor.
# Review: PO
safety-reason-schema-change = İfade şemayı değiştiriyor.
# Review: PO
safety-reason-table-lock = İfade tabloları bütünüyle kilitliyor.
# Review: PO
safety-reason-advisory-lock = İfade bir danışma kilidi (advisory lock) alıyor.
# Review: PO
safety-reason-external-io = İfade veritabanının dışındaki dosyalara ya da ağa erişiyor.
# Review: PO
safety-reason-prepared-statement = İfade, daha sonra çalışacak başka bir ifade hazırlıyor.
# Review: PO
safety-reason-notify = İfade diğer oturumlara bir bildirim gönderiyor.
# Review: PO
safety-reason-effects-unknown = İfadenin ne yaptığı belirlenemiyor.
# Review: PO
safety-reason-destructive = İfade çok sayıda veriyi değiştirebilir ya da silebilir.

# Review: PO
safety-destructive-no-where = { $table } için WHERE yok; bu yüzden her satır değişir.
# Review: PO
safety-destructive-always-true = { $table } üzerindeki WHERE koşulu her zaman doğru; bu yüzden her satır değişir.
# Review: PO
safety-destructive-join-without-on = Birleştirme { $table } satırlarını hiç sınırlamıyor; her satır değişebilir.
# Review: PO
safety-destructive-drop = { $table } kaldırılır.
# Review: PO
safety-destructive-drop-cascade = { $table } ve ona bağlı her şey kaldırılır.
# Review: PO
safety-destructive-truncate = { $table } içindeki bütün satırlar silinir.
# Review: PO
safety-destructive-drop-column = { $table } içindeki bir sütun, verisiyle birlikte kaldırılır.
# Review: PO
safety-destructive-drop-constraint = { $table } üzerindeki bir kısıt kaldırılır.
# Review: PO
safety-destructive-alter-type = { $table } içindeki bir sütunun türü değişir; veri yeniden yazılır.
# Review: PO
safety-destructive-merge-delete = MERGE, { $table } içinden satır siler.
# Review: PO
safety-destructive-update-limit-no-order = { $table } üzerinde ORDER BY olmadan UPDATE … LIMIT, satırları belirsiz bir sırayla değiştirir.
