/// The web origin (`scheme://host[:port]`) of an `http` or `https` URL; `None` for anything
/// else, such as `about:blank` or `file:`.
pub(crate) fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_and_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    (!host_and_port.is_empty())
        .then(|| format!("{scheme}://{}", host_and_port.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::origin_of;

    #[test]
    fn origins_keep_scheme_host_and_port_only() {
        assert_eq!(
            origin_of("https://GitHub.com/login?x=1").as_deref(),
            Some("https://github.com")
        );
        assert_eq!(
            origin_of("http://user:pw@localhost:8080/a").as_deref(),
            Some("http://localhost:8080")
        );
        assert_eq!(origin_of("about:blank"), None);
        assert_eq!(origin_of("file:///etc/passwd"), None);
    }
}
