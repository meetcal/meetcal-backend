| Route | Rust (ms) | RN client path (ms) | Rust vs RN % | User noticeable |
| --- | ---: | ---: | ---: | :---: |
| `GET /meets` | 28.95 | 27.48 | 5.3% | No |
| `GET /meets/:name` | 28.09 | 25.99 | 8.1% | No |
| `GET /meets/schedule/:name` | 31.01 | 24.69 | 25.6% | No |
| `GET /meets/athletes/:name` | 61.47 | 38.48 | 59.7% | No |
| `GET /clubs` | 29.18 | 28.40 | 2.7% | No |
| `GET /records` | 30.00 | 24.25 | 23.7% | No |
| `GET /wso` | 26.42 | 27.69 | -4.6% | No |
| `GET /wso-records` | 27.17 | 26.25 | 3.5% | No |
| `GET /standards` | 24.48 | 25.11 | -2.5% | No |
| **Total** (avg) | 31.86 | 27.59 | 13.5% | No |
