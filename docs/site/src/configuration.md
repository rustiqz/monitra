# Configuration

Monitra uses embedded defaults if both config files are absent. Effective precedence, lowest to highest, is XDG config, `./monitra.toml`, environment variables, then applicable CLI flags. `monitra start --config <PATH>` uses that file in place of `./monitra.toml`. Configuration commands write only the XDG file, never the project file.

The `k8s` array is assembled from both files (XDG entries followed by project entries); unlike provider fields, the second file does not replace the first array.

| Location | Purpose |
|---|---|
| `$XDG_CONFIG_HOME/monitra/config.toml` | User config; falls back to `$HOME/.config/monitra/config.toml`. |
| `./monitra.toml` | Optional project config. |
| `$XDG_DATA_HOME/monitra/monitra.db` | Default SQLite database; falls back to `$HOME/.local/share/monitra/monitra.db`. |

Both config files use the same TOML keys:

```toml
# Leave provider URLs absent to use embedded defaults.
api_token = "choose-a-secret-token"

[[k8s]]
name = "production"
kubeconfig = "/path/to/kubeconfig"
context = "production"
namespace = "default"
```

| Key | Meaning | Default |
|---|---|---|
| `store` | Store URL, such as `postgres://…` or `postgresql://…`. | Embedded SQLite. PostgreSQL is not yet implemented. |
| `cache` | Cache URL, such as `redis://…`. | In-process cache. Redis is not yet implemented. |
| `notifier` | `webhook://…` or `slack://…` target. | Log notifications. |
| `api_token` | Backend bearer token. | Generated and saved on first backend start, then printed once. |
| `k8s` | Array of cluster entries with `name`, `kubeconfig`, optional `context`, optional `namespace`. | No clusters. |

`MONITRA_STORE`, `MONITRA_CACHE`, `MONITRA_NOTIFIER`, and `MONITRA_API_TOKEN` override the corresponding file values. `monitra setup` prompts for Store, Cache, and Notifier URLs; blank answers keep defaults. `monitra service attach webhook://hooks.example.com/path` is another way to set a provider in the XDG file. Changes are read at daemon startup; restart to apply them.
