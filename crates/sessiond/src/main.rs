use anyhow::Result;
use waybroker_common::{ServiceBanner, ServiceRole};

fn main() -> Result<()> {
    let banner = ServiceBanner::new(ServiceRole::Sessiond, "lid, idle, suspend, session policy");
    println!("{}", banner.render());
    Ok(())
}
