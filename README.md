# elekter

Command line tool for automatically taking actions based on electricity prices in Estonia.

## Usage

```
USAGE:
    elekter [FLAGS] <config>

FLAGS:
    -n, --dry-run    Simulate current day, without executing any commands
    -h, --help       Prints help information
    -V, --version    Prints version information

ARGS:
    <config>    TOML configuration file
```

## Configuration

Configuration for actions are declared in TOML file. An example configuration might look as follows:

```toml
[package]
# Rated based on selected network package, specifying prices in EUR/MWh without VAT
day = 36.93548387096774 # Package 4 as of October 2025: Day Rate: 45.8 EUR/MWh with VAT (24%)
night = 20.967741935483872 # Package 4 as of October 2025: Night Rate: 26 EUR/MWh with VAT (24%)

[floor]
# Enabled floor heating always if price is below 25 EUR/MWh
threshold = 25.0
# Make sure that floor heating is enabled at least 50% of the time (during one day)
ratio = 0.5
cmd_on = [ 'curl', 'http://192.168.1.30/relay/0?turn=on' ]
cmd_off = [ 'curl', 'http://192.168.1.30/relay/0?turn=off' ]

[water]
# Enabled warm water heating always if price is below 25 EUR/MWh
threshold = 25.0
# Make sure that warm water heating is enabled at least 15% of the time for all 9 hour sliding windows of the day
ratio = 0.15
window = '9h'
cmd_on = [ 'curl', 'http://192.168.1.31/relay/0?turn=on' ]
cmd_off = [ 'curl', 'http://192.168.1.31/relay/0?turn=off' ]
```

There is no limitation of the amount of devices that can be configured, or the name which can given to them. Only the name `package` is reserved in order to configure the transmission rates charged by the electricity grid operator. The main parameters for each device are:

* `threshold`: If the price is equal or below `threshold`, the the device will always be switched on.
* `ratio`: For each sliding window within the day, the device will be turned on to reach the ratio.
* `window`: Defines the length of the sliding window for which the `ratio` is enforced. Only allowed if `ratio` is specified.
* `cmd_on`: Command (followed by any number of arguments) which is executed when the device is switched on.
* `cmd_off`: Command (followed by any number of arguments) which is executed when the device is switched off.

If the `window` parameter is not specified, the ratio will be enforced for the whole 24h window of the day.

Note that all prices are always without value added taxes (VAT) and in EUR/MWh.

## Authors

<a href="https://github.com/danwue/elekter/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=danwue/elekter" alt="contrib.rocks image" />
</a>

## Version History

* 0.0.1
    * Initial Release

## License

Distributed under the GPLv3 License. See `LICENSE.md` for more information.
