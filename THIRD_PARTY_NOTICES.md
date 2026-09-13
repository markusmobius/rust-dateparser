# Calendar Data Notices

[data/calendar-conversions.json](data/calendar-conversions.json) is copied
byte-for-byte from
[Go-DateParser v1.4.5](https://github.com/markusmobius/go-dateparser/blob/v1.4.5/internal/parser/calendars/data.json).
Its integer epoch-day boundaries were generated with `convertdate` 2.4.1 and
`hijridate` 2.6.0. Jalali year starts use `convertdate`'s Persian calculations;
Umm al-Qura month starts come from `hijridate`. The data records the generation
dependencies, imported source hashes and supported Gregorian bounds.

[src/non_gregorian.rs](src/non_gregorian.rs) performs native table lookup and
boundary arithmetic. No Python or Go calendar package is linked or bundled at
runtime. Calendar vocabulary in [data/calendars.json](data/calendars.json) comes
from Go-DateParser and is covered by the existing combined [LICENSE](LICENSE).
These notices supplement that file without replacing or splitting it.

`convertdate` uses PyMeeus 0.5.12 during external table generation. PyMeeus declares
LGPL-3.0-or-later; its source and license are available from
[PyMeeus](https://github.com/architest/pymeeus). Its code is not included in this
Rust implementation. Reproducing the numeric data requires the pinned generation
packages and their accompanying licenses.

## MIT Calendar Source Notices

`convertdate` 2.4.1: Copyright (c) 2014-2022 Neil Freeman.

`hijridate` 2.6.0: Copyright (c) Mohammed Alshehri.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.