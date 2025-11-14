use super::{expand_path, non_empty, KubernetesSection};
use anyhow::{Context, Result};
use kube::{
    config::{KubeConfigOptions, Kubeconfig},
    Client,
};

pub async fn build_kube_client(kube_cfg: &KubernetesSection) -> Result<Client> {
    let kubeconfig_path = expand_path(&kube_cfg.config_path);
    let kubeconfig = Kubeconfig::read_from(&kubeconfig_path).with_context(|| {
        format!(
            "Failed to read kubeconfig at '{}'",
            kubeconfig_path.display()
        )
    })?;

    let mut options = KubeConfigOptions::default();

    if let Some(ctx) = non_empty(&kube_cfg.context) {
        options.context = Some(ctx.to_owned());
    }
    if let Some(cluster) = non_empty(&kube_cfg.cluster_name) {
        options.cluster = Some(cluster.to_owned());
    }

    let mut client_config = kube::Config::from_custom_kubeconfig(kubeconfig, &options)
        .await
        .context("Failed to build Kubernetes configuration from kubeconfig file")?;

    if let Some(server) = non_empty(&kube_cfg.api_server) {
        client_config.cluster_url = server
            .parse()
            .context("Failed to parse 'kubernetes.api_server' as a URL")?;
    }

    Client::try_from(client_config)
        .context("Failed to construct Kubernetes client from configuration")
}
