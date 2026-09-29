# First-load grid examples (design artifact, not product code)

Rule (01-design.md section 3.1): sort confirmed terminal members by stable pane ID (numeric-aware compare, ties by UTF-16 code-unit order); `cols = ceil(sqrt(N))`, `rows = ceil(N / cols)`, `base = floor(N / rows)`, `extra = N mod rows`; the first `extra` rows hold `base + 1` terminals and the rest hold `base`; fill row-major in sorted order. Every split child gets weight 1 (equal shares within its row/column). `N = 1` is the single leaf.

The approved demo's `buildGrid` (`demo.html:203-212`) chunks row-major by `cols` and lets the last row be short. Both rules give the same shapes for N = 1-6, 8, 9, 11, 12, 15, 16. They differ only where the demo's last row would hold fewer than `base` terminals (N = 7, 10, 13, 14, 17). The design uses the even-row rule there so no row is left with a single wide terminal while others are narrow.

| N | cols | rows | terminals per row (design) | demo `buildGrid` rows |
|---|------|------|----------------------------|-----------------------|
| 1 | 1 | 1 | 1 | 1 |
| 2 | 2 | 1 | 2 (side by side) | 2 |
| 3 | 2 | 2 | 2, 1 | 2, 1 |
| 4 | 2 | 2 | 2, 2 | 2, 2 |
| 5 | 3 | 2 | 3, 2 | 3, 2 |
| 6 | 3 | 2 | 3, 3 | 3, 3 |
| 7 | 3 | 3 | 3, 2, 2 | 3, 3, 1 |
| 8 | 3 | 3 | 3, 3, 2 | 3, 3, 2 |
| 9 | 3 | 3 | 3, 3, 3 | 3, 3, 3 |
| 10 | 4 | 3 | 4, 3, 3 | 4, 4, 2 |
| 11 | 4 | 3 | 4, 4, 3 | 4, 4, 3 |
| 12 | 4 | 3 | 4, 4, 4 | 4, 4, 4 |
| 13 | 4 | 4 | 4, 3, 3, 3 | 4, 4, 4, 1 |
| 14 | 4 | 4 | 4, 4, 3, 3 | 4, 4, 4, 2 |
| 15 | 4 | 4 | 4, 4, 4, 3 | 4, 4, 4, 3 |
| 16 | 4 | 4 | 4, 4, 4, 4 | 4, 4, 4, 4 |
| 17 | 5 | 4 | 5, 4, 4, 4 | 5, 5, 5, 2 |

(The design column was computed by hand from the formula above and cross-checked against the demo column for the equal cases; no code was run.)

## Shapes (numbers are sorted-order positions)

```
N=2                 N=3                 N=4
+--------+--------+ +--------+--------+ +--------+--------+
|   1    |   2    | |   1    |   2    | |   1    |   2    |
|        |        | +--------+--------+ +--------+--------+
|        |        | |        3         | |   3    |   4    |
+--------+--------+ +-------------------+ +--------+--------+

N=5                          N=7 (design)
+------+------+------+       +------+------+------+
|  1   |  2   |  3   |       |  1   |  2   |  3   |
+------+------+------+       +------+------+------+
|    4      |    5    |      |     4     |    5    |
+-----------+---------+      +-----------+---------+
                             |     6     |    7    |
                             +-----------+---------+
```

Tree form of N = 5: `col{ row{1,2,3}, row{4,5} }`, all weights 1.
