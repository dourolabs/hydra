use super::{KubernetesSection, expand_path, non_empty};
use anyhow::{Context, Result};
use kube::{
    Client,
    config::{KubeConfigOptions, Kubeconfig},
};

pub async fn build_kube_client(kube_cfg: &KubernetesSection) -> Result<Client> {
    let mut client_config = if kube_cfg.in_cluster {
        kube::Config::infer()
            .await
            .context("Failed to infer in-cluster Kubernetes configuration. Is the server running inside a cluster with a valid ServiceAccount?")?
    } else {
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

        kube::Config::from_custom_kubeconfig(kubeconfig, &options)
            .await
            .context("Failed to build Kubernetes configuration from kubeconfig file")?
    };

    if let Some(server) = non_empty(&kube_cfg.api_server) {
        client_config.cluster_url = server
            .parse()
            .context("Failed to parse 'kubernetes.api_server' as a URL")?;
    }

    Client::try_from(client_config)
        .context("Failed to construct Kubernetes client from configuration")
}
