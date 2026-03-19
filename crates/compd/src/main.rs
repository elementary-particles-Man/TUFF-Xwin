use anyhow::Result;
use waybroker_common::{ServiceBanner, ServiceRole};

fn main() -> Result<()> {
    let banner = ServiceBanner::new(ServiceRole::Compd, "scene, focus, composition policy");
    println!("{}", banner.render());
    Ok(())
}
