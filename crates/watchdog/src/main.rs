use anyhow::Result;
use waybroker_common::{ServiceBanner, ServiceRole};

fn main() -> Result<()> {
    let banner = ServiceBanner::new(ServiceRole::Watchdog, "display stack recovery control");
    println!("{}", banner.render());
    Ok(())
}
