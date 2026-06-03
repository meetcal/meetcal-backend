| Route | Rust (ms) | RN client path (ms) | Rust vs RN % | User noticeable |
| --- | ---: | ---: | ---: | :---: |
| `GET /meets` | 27.28 | 27.37 | -0.3% | No |
| `GET /meets/:name` | 28.09 | 25.99 | 8.1% | No |
| `GET /meets/schedule/:name` | 31.01 | 24.69 | 25.6% | No |
| `GET /meets/athletes/:name` | 61.47 | 38.48 | 59.7% | No |
| `GET /clubs` | 28.73 | 25.67 | 11.9% | No |
| `GET /records` | 27.14 | 26.07 | 4.1% | No |
| `GET /wso` | 25.69 | 25.87 | -0.7% | No |
| `GET /wso-records` | 26.68 | 25.68 | 3.9% | No |
| `GET /standards` | 27.29 | 24.86 | 9.8% | No |
| `GET /qualifying-totals` | 27.16 | 24.13 | 12.6% | No |
| `GET /intl-rankings` | 25.87 | 24.04 | 7.6% | No |
| `GET /meet-details` | 26.37 | 24.15 | 9.2% | No |
| `GET /meets/schedule` | 30.17 | 27.30 | 10.5% | No |
| `GET /meets/athletes` | 58.06 | 35.12 | 65.3% | No |
| `GET /nat-rankings` | 30.31 | 30.00 | 1.0% | No |
| `GET /adaptive` | 46.14 | 30.35 | 52.0% | No |
| `GET /search` | 32.40 | 24.67 | 31.3% | No |
| **Total** (avg) | 32.93 | 27.32 | 18.3% | No |
