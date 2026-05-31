| Route | Rust (ms) | RN client path (ms) | Rust vs RN % | User noticeable |
| --- | ---: | ---: | ---: | :---: |
| `GET /meets` | 27.17 | 27.86 | -2.5% | No |
| `GET /meets/:name` | 25.78 | 25.41 | 1.5% | No |
| `GET /meets/schedule/:name` | 28.78 | 24.26 | 18.6% | No |
| `GET /meets/athletes/:name` | 57.77 | 37.00 | 56.1% | No |
| **Total** (avg) | 34.88 | 28.63 | 18.4% | No |
