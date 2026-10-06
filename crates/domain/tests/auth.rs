use domain::auth::{Auth, effective};

#[test]
fn own_auth_wins_else_collection_else_none() {
    let own = Auth::Bearer {
        token: "own".into(),
    };
    let coll = Auth::Basic {
        username: "u".into(),
        password: "p".into(),
    };
    assert_eq!(effective(Some(&own), Some(&coll)), Some(&own));
    assert_eq!(effective(None, Some(&coll)), Some(&coll));
    assert_eq!(effective(None, None), None);
    assert!(!Auth::Unsupported("oauth2".into()).is_sendable());
    assert!(own.is_sendable());
}
