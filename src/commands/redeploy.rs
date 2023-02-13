pub(crate) fn redeploy() -> Result<(), anyhow::Error> {
    super::undeploy(None::<&str>)?;
    super::deploy(None::<&str>)?;
    Ok(())
}
