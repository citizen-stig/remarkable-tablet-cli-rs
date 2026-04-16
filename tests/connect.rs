use remarkable_tablet_cli_rs::connection::FakeConnection;
use remarkable_tablet_cli_rs::tablet::fetch_device_info;

#[tokio::test]
async fn reports_device_info_over_fake_connection() {
    let conn = FakeConnection::new();
    conn.set_file("/etc/version", "20230412102300\n");
    conn.mkdir("/sys/class/power_supply/max77818_battery");
    conn.set_file("/sys/class/power_supply/max77818_battery/capacity", "82\n");
    conn.set_command_output(
        "df -k",
        "Filesystem     1K-blocks    Used Available Use% Mounted on\n\
         /dev/root        6291456 2097152   4194304  33% /\n",
    );

    let info = fetch_device_info(
        &conn,
        "10.11.99.1",
        "/home/root/.local/share/remarkable/xochitl",
    )
    .await
    .expect("fetch_device_info succeeds");

    assert_eq!(info.host, "10.11.99.1");
    assert_eq!(info.connection_type, "usb");
    assert_eq!(info.firmware_version, "20230412102300");
    assert_eq!(info.battery_percent, 82);
    assert_eq!(info.disk_total_mb, 6144);
    assert_eq!(info.disk_used_mb, 2048);
    assert_eq!(info.disk_free_mb, 4096);
}

#[tokio::test]
async fn fails_gracefully_without_battery() {
    let conn = FakeConnection::new();
    conn.set_file("/etc/version", "1.0");
    conn.mkdir("/sys/class/power_supply");
    conn.set_command_output(
        "df -k",
        "Filesystem 1K-blocks Used Available Use% Mounted on\n/dev/root 100 50 50 50% /\n",
    );

    let err = fetch_device_info(&conn, "10.11.99.1", "/any")
        .await
        .expect_err("should fail: no battery entries");
    assert!(format!("{err}").contains("no battery"));
}
