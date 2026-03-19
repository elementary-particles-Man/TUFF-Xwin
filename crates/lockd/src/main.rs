use anyhow::Result;
use waybroker_common::{ServiceBanner, ServiceRole};

fn main() -> Result<()> {
    let banner = ServiceBanner::new(ServiceRole::Lockd, "lockscreen and auth ui");
    println!("{}", banner.render());
    Ok(())
}
