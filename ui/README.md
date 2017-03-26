## Configuration

| Key                                   | Required | Default Value               | Description                                                             |
|---------------------------------------|----------|-----------------------------|-------------------------------------------------------------------------|
| `PLAYGROUND_UI_ROOT`                  | **Yes**  |                             | The path to the HTML, CSS, and Javascript files                         |
| `PLAYGROUND_UI_LISTEN_HOST`           | No       | 127.0.0.1                   | The host or IP address to listen on                                     |
| `PLAYGROUND_UI_PUBLIC_HOST`           | No       | `PLAYGROUND_UI_LISTEN_HOST` | The host or IP address that public-facing URLS will use                 |
| `PLAYGROUND_UI_HTTP_PORT`             | No       | 5000                        | The port to listen on for HTTP traffic                                  |
| `PLAYGROUND_UI_HTTPS_PORT`            | No       | 5001                        | The port to listen on for HTTPS traffic                                 |
| `PLAYGROUND_UI_HTTPS_PKCS12_FILE`     | No       |                             | The path to a PKCS12 identity file                                      |
| `PLAYGROUND_UI_HTTPS_PKCS12_PASSWORD` | No       |                             | The password to unlock the PKCS12 identity file                         |
| `PLAYGROUND_LOG_FILE`                 | No       | access-log.csv              | The file to record accesses                                             |
| `TMPDIR`                              | No       | system-provided             | Where compilation artifacts will be saved. Must be accessible to Docker |
