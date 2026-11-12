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
# Changed from the approved mock text: M2a has no certificate review, only
# "Encrypt without verifying" (R3). M2b restores the review flow.
error-conn-tls-untrusted-body = Sunucunun sertifikası, bu Mac'in güvendiği bir makam tarafından imzalanmamış. Bağlantıyı sertifikayı doğrulamadan şifreleyebilir ya da yöneticinizden CA'yı isteyebilirsiniz.

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
