//! 營運人員的維運指令(`tenant-saas admin …`)。
mod common;

use axum::http::{Method, StatusCode};
use common::{add_member, app, call, create_tenant, signup};
use sqlx::PgPool;
use tenant_saas::admin_cli::run;

async fn cli(pool: &PgPool, args: &[&str]) -> anyhow::Result<String> {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    run(pool, &args, &mut out).await?;
    Ok(String::from_utf8(out)?)
}

#[sqlx::test]
async fn plans_and_shops_are_listed_with_usage(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    signup(&app, "b@example.com").await;
    add_member(&pool, "shop-a", "b@example.com", "staff").await;
    let other = signup(&app, "c@example.com").await;
    create_tenant(&app, &other, "shop-b").await;

    let plans = cli(&pool, &["plans"]).await.unwrap();
    assert!(plans.contains("free\t免費版\t0\t2\t5\t50"), "{plans}");
    assert!(
        plans.contains("business\t企業版\t1999\t不限\t不限\t不限"),
        "{plans}"
    );

    let shops = cli(&pool, &["shops"]).await.unwrap();
    let a = shops.lines().find(|l| l.starts_with("shop-a\t")).unwrap();
    let cols: Vec<&str> = a.split('\t').collect();
    assert_eq!(&cols[2..6], ["free", "active", "2", "0"], "{a}"); // 方案、狀態、成員 2、預約 0
    assert!(shops.contains("共 2 家"), "{shops}");
    // 數量限制
    assert!(
        cli(&pool, &["shops", "1"])
            .await
            .unwrap()
            .contains("共 1 家")
    );
    assert!(cli(&pool, &["shops", "0"]).await.is_err());
    assert!(cli(&pool, &["shops", "abc"]).await.is_err());
    assert!(cli(&pool, &["shops", "100000"]).await.is_err());

    let show = cli(&pool, &["show", "shop-a"]).await.unwrap();
    assert!(show.contains("店主:a@example.com"), "{show}");
    assert!(show.contains("成員:2"), "{show}");
    assert!(show.contains("沒有綁定 Stripe"), "{show}");
    // 申請刪除的會標出來
    sqlx::query("UPDATE tenants SET deletion_requested_at = now(), deletion_scheduled_at = now() + interval '30 days' WHERE slug = 'shop-a'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        cli(&pool, &["show", "shop-a"])
            .await
            .unwrap()
            .contains("已申請刪除")
    );
    let shops = cli(&pool, &["shops"]).await.unwrap();
    let a = shops.lines().find(|l| l.starts_with("shop-a\t")).unwrap();
    assert_ne!(a.split('\t').next_back().unwrap(), "-", "{a}");
}

#[sqlx::test]
async fn changing_the_plan_takes_effect_and_is_audited(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    let plan_of = |app: axum::Router, owner: String| async move {
        let (_, body) = call(&app, Method::GET, "/t/shop-a/plan", None, Some(&owner)).await;
        body["plan"]["id"].as_str().unwrap_or_default().to_string()
    };
    assert_eq!(plan_of(app.clone(), owner.clone()).await, "free");

    let out = cli(&pool, &["set-plan", "shop-a", "pro"]).await.unwrap();
    assert!(out.contains("free → pro"), "{out}");
    assert_eq!(plan_of(app.clone(), owner.clone()).await, "pro");
    // 沒有變更就說沒有變更,而且不留稽核
    assert!(
        cli(&pool, &["set-plan", "shop-a", "pro"])
            .await
            .unwrap()
            .contains("沒有變更")
    );

    // 稽核:店主在稽核頁看得到,是「系統」做的
    let (status, logs) = call(
        &app,
        Method::GET,
        "/t/shop-a/audit-logs?action=operator.plan_changed",
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{logs}");
    let items = logs["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{logs}");
    assert_eq!(items[0]["actor_type"], "system");
    assert_eq!(
        items[0]["detail"],
        serde_json::json!({"from": "free", "to": "pro"})
    );
}

#[sqlx::test]
async fn suspending_locks_everyone_out_until_it_is_lifted(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    let me = |app: axum::Router, owner: String| async move {
        call(&app, Method::GET, "/t/shop-a/me", None, Some(&owner))
            .await
            .0
    };
    let public = |app: axum::Router| async move {
        call(&app, Method::GET, "/public/shops/shop-a", None, None)
            .await
            .0
    };
    assert_eq!(me(app.clone(), owner.clone()).await, StatusCode::OK);
    assert_eq!(public(app.clone()).await, StatusCode::OK);

    assert!(
        cli(&pool, &["suspend", "shop-a"])
            .await
            .unwrap()
            .contains("active → suspended")
    );
    assert_eq!(me(app.clone(), owner.clone()).await, StatusCode::NOT_FOUND);
    assert_eq!(public(app.clone()).await, StatusCode::NOT_FOUND);
    // 資料沒動:只是進不去
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memberships")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert!(
        cli(&pool, &["suspend", "shop-a"])
            .await
            .unwrap()
            .contains("沒有變更")
    );

    assert!(
        cli(&pool, &["unsuspend", "shop-a"])
            .await
            .unwrap()
            .contains("suspended → active")
    );
    assert_eq!(me(app.clone(), owner.clone()).await, StatusCode::OK);
    assert_eq!(public(app.clone()).await, StatusCode::OK);
    let actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_logs WHERE action LIKE 'operator.%' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(actions, ["operator.suspended", "operator.unsuspended"]);
}

#[sqlx::test]
async fn bad_input_is_rejected_before_touching_the_database(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;

    for bad in [
        "x",
        "A-B",
        "-bad",
        "bad-",
        "a'; DROP TABLE tenants;--",
        "有中文",
        &"a".repeat(41),
    ] {
        let err = cli(&pool, &["show", bad]).await.unwrap_err().to_string();
        assert!(err.contains("格式不正確"), "{bad}: {err}");
    }
    assert!(
        cli(&pool, &["show", "no-such-shop"])
            .await
            .unwrap_err()
            .to_string()
            .contains("找不到店家")
    );
    let err = cli(&pool, &["set-plan", "shop-a", "platinum"])
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("沒有這個方案") && err.contains("free"),
        "{err}"
    );
    // 方案不存在時什麼都沒改
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT plan_id FROM tenants")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "free"
    );
    // 用法:未知指令、缺參數、多餘參數
    for args in [
        &[][..],
        &["nope"],
        &["show"],
        &["show", "shop-a", "extra"],
        &["set-plan", "shop-a"],
        &["set-plan", "shop-a", "pro", "extra"],
        &["unsuspend", "shop-a", "extra"],
        &["suspend", "shop-a", "extra"],
        &["plans", "extra"],
    ] {
        let err = cli(&pool, args).await.unwrap_err().to_string();
        assert!(err.contains("用法"), "{args:?}: {err}");
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tenants")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
}

#[sqlx::test]
async fn old_audit_logs_can_be_archived_as_json_lines_without_deleting_anything(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(before >= 1);
    // 一部分挪到很久以前
    sqlx::query("UPDATE audit_logs SET created_at = now() - interval '800 days'")
        .execute(&pool)
        .await
        .unwrap();
    let recent_before: i64 = {
        // 再造一筆近期的:不該被輸出
        sqlx::query(
            "INSERT INTO audit_logs (tenant_id, actor_type, action, entity_type, detail)
             SELECT id, 'system', 'recent.thing', 'tenant', '{}' FROM tenants",
        )
        .execute(&pool)
        .await
        .unwrap();
        1
    };

    let out = cli(&pool, &["export-audit", "700"]).await.unwrap();
    let lines: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).expect("每一行都是合法的 JSON"))
        .collect();
    assert_eq!(lines.len() as i64, before, "{out}");
    assert!(lines.iter().all(|l| l["action"] != "recent.thing"));
    assert_eq!(lines[0]["tenant_slug"], "shop-a");
    assert!(lines[0]["created_at"].is_string() && lines[0]["detail"].is_object());
    // 由舊到新、id 嚴格遞增
    let ids: Vec<i64> = lines.iter().map(|l| l["id"].as_i64().unwrap()).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]));
    // 只讀:什麼都沒刪
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(after, before + recent_before);
    // 天數太長:沒有符合的
    assert_eq!(cli(&pool, &["export-audit", "900"]).await.unwrap(), "");
    // 參數檢查
    for bad in ["0", "-1", "abc", "99999"] {
        assert!(cli(&pool, &["export-audit", bad]).await.is_err(), "{bad}");
    }
    assert!(cli(&pool, &["export-audit"]).await.is_err());
    assert!(cli(&pool, &["export-audit", "700", "extra"]).await.is_err());
}

#[sqlx::test]
async fn archiving_more_than_one_batch_loses_and_duplicates_nothing(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    sqlx::query(
        "INSERT INTO audit_logs (tenant_id, actor_type, action, entity_type, detail, created_at)
         SELECT t.id, 'system', 'bulk.' || g, 'tenant', '{}', now() - interval '900 days'
         FROM tenants t, generate_series(1, 5200) g",
    )
    .execute(&pool)
    .await
    .unwrap();
    let expected: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE created_at < now() - interval '700 days'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(expected > 5000, "要超過一批(每批 5000 筆)");

    let out = cli(&pool, &["export-audit", "700"]).await.unwrap();
    let ids: Vec<i64> = out
        .lines()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["id"]
                .as_i64()
                .unwrap()
        })
        .collect();
    assert_eq!(ids.len() as i64, expected);
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "不重複、依序");
}
