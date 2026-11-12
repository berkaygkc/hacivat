// Typed message IDs reject wrong arguments at compile time (I18N-04 step 1, test 17).
use hacivat_l10n::ids::cm;
use hacivat_l10n::{Count, L10n, NoArgs, Runtime, Text};

fn main() {
    let l = L10n::new(Runtime::default());
    // A message without variables takes no arguments struct.
    let _ = l.t(cm::BRAND_HACIVAT_CAPS, cm::CmErrorUnknown { code: Text("x".into()) });
    // A message with variables needs its own arguments struct.
    let _ = l.t(cm::ERROR_UNKNOWN, NoArgs);
    // `$code (String)` is text, not a count.
    let _ = l.t(cm::ERROR_UNKNOWN, cm::CmErrorUnknown { code: Count(1) });
    // There is no `$name` on `cm-error-unknown`.
    let _ = l.t(cm::ERROR_UNKNOWN, cm::CmErrorUnknown { name: Text("x".into()) });
}
