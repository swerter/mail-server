/*
 * Copyright (c) 2023 Stalwart Labs Ltd.
 *
 * This file is part of Stalwart Mail Server.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of
 * the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 * in the LICENSE file at the top-level directory of this distribution.
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 *
 * You can be released from the requirements of the AGPLv3 license by
 * purchasing a commercial license. Please contact licensing@stalw.art
 * for more details.
*/

use std::fmt::Debug;

use directory::{backend::internal::manage::ManageDirectory, Principal, QueryBy, Type};
use mail_send::Credentials;

use crate::directory::{map_account_ids, DirectoryTest};

#[tokio::test]
async fn ldap_directory() {
    // Enable logging
    /*tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_max_level(tracing::Level::DEBUG)
            .finish(),
    )
    .unwrap();*/

    // Obtain directory handle
    let mut config = DirectoryTest::new("sqlite".into()).await;
    let handle = config.directories.directories.remove("ldap").unwrap();
    let base_store = config.stores.stores.get("sqlite").unwrap();
    let core = config.core;

    // Test authentication
    assert_eq!(
        handle
            .query(
                QueryBy::Credentials(&Credentials::Plain {
                    username: "john".to_string(),
                    secret: "12345".to_string()
                }),
                true
            )
            .await
            .unwrap()
            .unwrap()
            .into_sorted(),
        Principal {
            id: base_store.get_account_id("john").await.unwrap().unwrap(),
            name: "john".to_string(),
            description: "John Doe".to_string().into(),
            secrets: vec!["12345".to_string()],
            typ: Type::Individual,
            member_of: map_account_ids(base_store, vec!["sales"]).await,
            emails: vec![
                "john@example.org".to_string(),
                "john.doe@example.org".to_string()
            ],
            ..Default::default()
        }
        .into_sorted()
    );
    assert_eq!(
        handle
            .query(
                QueryBy::Credentials(&Credentials::Plain {
                    username: "bill".to_string(),
                    secret: "password".to_string()
                }),
                true
            )
            .await
            .unwrap()
            .unwrap()
            .into_sorted(),
        Principal {
            id: base_store.get_account_id("bill").await.unwrap().unwrap(),
            name: "bill".to_string(),
            description: "Bill Foobar".to_string().into(),
            secrets: vec![
                "$2y$05$bvIG6Nmid91Mu9RcmmWZfO5HJIMCT8riNW0hEp8f6/FuA2/mHZFpe".to_string()
            ],
            typ: Type::Individual,
            quota: 500000,
            emails: vec!["bill@example.org".to_string(),],
            ..Default::default()
        }
        .into_sorted()
    );
    assert!(handle
        .query(
            QueryBy::Credentials(&Credentials::Plain {
                username: "bill".to_string(),
                secret: "invalid".to_string()
            }),
            true
        )
        .await
        .unwrap()
        .is_none());

    // Get user by name
    assert_eq!(
        handle
            .query(QueryBy::Name("jane"), true)
            .await
            .unwrap()
            .unwrap()
            .into_sorted(),
        Principal {
            id: base_store.get_account_id("jane").await.unwrap().unwrap(),
            name: "jane".to_string(),
            description: "Jane Doe".to_string().into(),
            typ: Type::Individual,
            secrets: vec!["abcde".to_string()],
            member_of: map_account_ids(base_store, vec!["sales", "support"]).await,
            emails: vec!["jane@example.org".to_string(),],
            ..Default::default()
        }
        .into_sorted()
    );

    // Get group by name
    assert_eq!(
        handle
            .query(QueryBy::Name("sales"), true)
            .await
            .unwrap()
            .unwrap(),
        Principal {
            id: base_store.get_account_id("sales").await.unwrap().unwrap(),
            name: "sales".to_string(),
            description: "sales".to_string().into(),
            typ: Type::Group,
            ..Default::default()
        }
    );

    // Ids by email
    compare_sorted(
        core.email_to_ids(&handle, "jane@example.org")
            .await
            .unwrap(),
        map_account_ids(base_store, vec!["jane"]).await,
    );
    compare_sorted(
        core.email_to_ids(&handle, "jane+alias@example.org")
            .await
            .unwrap(),
        map_account_ids(base_store, vec!["jane"]).await,
    );
    compare_sorted(
        core.email_to_ids(&handle, "info@example.org")
            .await
            .unwrap(),
        map_account_ids(base_store, vec!["bill", "jane", "john"]).await,
    );
    compare_sorted(
        core.email_to_ids(&handle, "info+alias@example.org")
            .await
            .unwrap(),
        map_account_ids(base_store, vec!["bill", "jane", "john"]).await,
    );
    compare_sorted(
        core.email_to_ids(&handle, "unknown@example.org")
            .await
            .unwrap(),
        Vec::<u32>::new(),
    );
    assert_eq!(
        core.email_to_ids(&handle, "anything@catchall.org")
            .await
            .unwrap(),
        map_account_ids(base_store, vec!["robert"]).await
    );

    // Domain validation
    assert!(handle.is_local_domain("example.org").await.unwrap());
    assert!(!handle.is_local_domain("other.org").await.unwrap());

    // RCPT TO
    assert!(core.rcpt(&handle, "jane@example.org").await.unwrap());
    assert!(core.rcpt(&handle, "info@example.org").await.unwrap());
    assert!(core.rcpt(&handle, "jane+alias@example.org").await.unwrap());
    assert!(core.rcpt(&handle, "info+alias@example.org").await.unwrap());
    assert!(core
        .rcpt(&handle, "random_user@catchall.org")
        .await
        .unwrap());
    assert!(!core.rcpt(&handle, "invalid@example.org").await.unwrap());

    // VRFY
    compare_sorted(
        core.vrfy(&handle, "jane").await.unwrap(),
        vec!["jane@example.org".to_string()],
    );
    compare_sorted(
        core.vrfy(&handle, "john").await.unwrap(),
        vec!["john@example.org".to_string()],
    );
    compare_sorted(
        core.vrfy(&handle, "jane+alias@example").await.unwrap(),
        vec!["jane@example.org".to_string()],
    );
    compare_sorted(
        core.vrfy(&handle, "info").await.unwrap(),
        Vec::<String>::new(),
    );
    compare_sorted(
        core.vrfy(&handle, "invalid").await.unwrap(),
        Vec::<String>::new(),
    );

    // EXPN
    compare_sorted(
        core.expn(&handle, "info@example.org").await.unwrap(),
        vec![
            "bill@example.org".to_string(),
            "jane@example.org".to_string(),
            "john@example.org".to_string(),
        ],
    );
    compare_sorted(
        core.expn(&handle, "john@example.org").await.unwrap(),
        Vec::<String>::new(),
    );
}

fn compare_sorted<T: Eq + Debug>(v1: Vec<T>, v2: Vec<T>) {
    for val in v1.iter() {
        assert!(v2.contains(val), "{v1:?} != {v2:?}");
    }

    for val in v2.iter() {
        assert!(v1.contains(val), "{v1:?} != {v2:?}");
    }
}
