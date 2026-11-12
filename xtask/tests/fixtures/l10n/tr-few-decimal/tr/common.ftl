### Ortak metinler.

cm-ok = Tamam

cm-rows =
    { $count ->
        [one] { $count } satır
        [few] { $count } satır
       *[other] { $count } satır
    }

cm-hint = Yenilemek için <code>F5</code> tuşuna basın.

cm-saved = { $name } bağlantısı kaydedildi.
    .title = Kaydedildi
