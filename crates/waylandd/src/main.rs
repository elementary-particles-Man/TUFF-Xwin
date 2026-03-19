use anyhow::Result;
use waybroker_common::{ServiceBanner, ServiceRole};

fn main() -> Result<()> {
    let banner = ServiceBanner::new(
        ServiceRole::Waylandd,
        "wayland endpoint, client lifecycle, clipboard core",
    );
    println!("{}", banner.render());
    Ok(())
}
