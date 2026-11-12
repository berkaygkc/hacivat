//! Fixture for the code scan; never compiled.
fn view(l10n: &L10n, name: &str) {
    let ok = l10n.text(ids::cm::OK);
    let bad = l10n.text(ids::cm::NO_SUCH_MESSAGE);
    let by_literal = l10n.t_dynamic("hv-no-such-literal");
    let joined = format!("{} {}", l10n.t(DELETE), name);
    row.child("Hard coded");
    row.child("Allowed text");
    row.child("…");
}

#[cfg(test)]
mod tests {
    fn only_in_tests() { row.child("Test only text"); }
}
