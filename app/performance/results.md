| Route | Rust (ms) | RN client path (ms) | Rust vs RN % | User noticeable |
| --- | ---: | ---: | ---: | :---: |
| `GET /meets` | 24.385 | 30.04 | -18.8% | No |
| `GET /meets/:name` | 23.19 | 29.96 | -22.6% | No |
| `GET /meets/schedule/:name` | 36.345 | 27.84 | 30.5% | No |
| **Total** (avg) | 27.97 | 29.28 | -3.6% | No |
