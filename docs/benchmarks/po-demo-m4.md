# M4 PO demosu

Bu dosyanın sahibi Görev 33'tür (kapı). Aşağıdaki blok Görev 29'un PO maddesidir; Görev 33 onu
demo adımı 1b'nin yanına taşır.

## m4-t29-g1-bench

**PO maddesi: G1 şifreli varyant, gerçek Keychain okuması.**

`catalog_cold` bench'inin şifreli varyantı (`g1-catalog-cold-encrypted`) bellek içi bir sır deposu
kullanır ve Keychain'e hiç dokunmaz. Bu yüzden ölçülen şifreli süre, gerçek Keychain okumasını
içermez. Uygulamanın ilk erişimde gösterebileceği Keychain izin isteği de bu süreye girmez.
`--key-delay-ms N` yalnızca bir varsayımdır. Görev 29'un ölçümünde N = 0 idi ve süre p95 147,3 ms
çıktı (yükü yüksek bir makinede, yetkili değil).

PO'dan istenen:

1. Demo adımı 1b'de (şifreli, önbellekli bir production profili açık geri yüklenir), Keychain'in
   `CatalogKey` okumasının ne kadar sürdüğünü ölçün. İlk açılış (izin isteğiyle) ve sonraki
   açılışlar ayrı ayrı yazılır. Uygulamanın dışından da ölçebilirsiniz:
   `time security find-generic-password -s 'hacivat/<profil uuid>/catalog-key' -a hacivat`
   (debug derlemede servis öneki `hacivat-dev`). Bu komut sırrın kendisini çıktıya yazmaz,
   çünkü `-w` / `-g` verilmez.
2. Ölçülen gecikmeyle bench'i yeniden çalıştırın:
   `cargo run --release -p hacivat-app --features bench --example catalog_cold -- --fixture target/bench/g1-160k.v1.sqlite --runs 10 --offline --variant encrypted --key-delay-ms <ölçülen ms>`
3. Şifreli p95 300 ms'yi aşarsa bu bir PO kararıdır. Seçenekler: kabul etmek, anahtarı açılıştan
   önce okumak, ya da şifreli profilde ağacı anahtar gelene kadar kilitli göstermek (bugünkü
   davranış: anahtar okunurken arayüz bloklanmaz). Karar buraya yazılır, `m4.md`'de gizlenmez.

| Ölçüm | Değer |
|---|---|
| Keychain `CatalogKey` okuması, ilk açılış (izin isteğiyle) | |
| Keychain `CatalogKey` okuması, sonraki açılışlar | |
| `g1-catalog-cold-encrypted` p95, `--key-delay-ms` ölçülen değerle | |
| Karar | |
