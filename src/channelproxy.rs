use log;
use pwm_pca9685::Channel;

use crate::{
    ChannelConfig, ChannelCountLimits, ChannelProxy, Pca9685Error, Pca9685Proxy, Pca9685Result,
    DEFAULT_CHANNEL_COUNT_LIMITS, PCA_PWM_RESOLUTION,
};

impl ChannelProxy {
    pub fn new(channel: Channel, pca_count_length_ms: f64, pca_max_pw_ms: f64) -> ChannelProxy {
        ChannelProxy {
            name: String::from(format!("Channel {:?}", channel)),
            config: ChannelConfig {
                channel: channel,
                current_count: None,
                custom_limits: None,
            },
            pca_count_length_ms: pca_count_length_ms,
            pca_max_pw_ms: pca_max_pw_ms,
        }
    }

    pub fn configure(&mut self, config: &ChannelConfig) -> ChannelConfig {
        self.configure_limits(&config.custom_limits)
    }

    pub fn config(&self) -> ChannelConfig {
        ChannelConfig {
            channel: self.config.channel,
            current_count: match self.config.current_count {
                Some(pwm_count) => Some(pwm_count),
                None => None,
            },
            custom_limits: match &self.config.custom_limits {
                Some(limits) => Some(ChannelCountLimits {
                    min_on_count: limits.min_on_count,
                    max_on_count: limits.max_on_count,
                }),
                None => None,
            },
        }
    }

    pub fn configure_limits(
        &mut self,
        custom_limits: &Option<ChannelCountLimits>,
    ) -> ChannelConfig {
        match custom_limits {
            Some(limits) => {
                log::info!(
                    target: &self.name,
                    "Configured limits to [{}, {})", limits.min_on_count, limits.max_on_count
                );
                self.config.custom_limits = Some(ChannelCountLimits {
                    max_on_count: limits.max_on_count,
                    min_on_count: limits.min_on_count,
                })
            }
            None => {
                log::info!(target: &self.name, "Configured limits to None");
                self.config.custom_limits = None
            }
        }

        self.config()
    }

    // fn configure_limits_ms(&mut self, min_pw_ms: f64, max_pw_ms: f64) -> &ChannelConfig {
    //     self.configure_limits(&Some(ChannelCountLimits {
    //         min_on_count: self.pw_to_count(min_pw_ms).unwrap(), // TODO Don't use unwrap
    //         max_on_count: self.pw_to_count(max_pw_ms).unwrap(),
    //     }));

    //     &self.config
    // }

    pub fn full_on(&mut self, pca: &mut Box<dyn Pca9685Proxy>) -> Pca9685Result<ChannelConfig> {
        self.config.current_count = Some(PCA_PWM_RESOLUTION);

        log::info!(target: &self.name, "Setting output to FULL ON");

        match pca.set_channel_full_on(self.config.channel) {
            Ok(()) => Ok(self.config()),
            Err(error) => Err(Pca9685Error::Pca9685DriverError(error)),
        }
    }

    pub fn full_off(&mut self, pca: &mut Box<dyn Pca9685Proxy>) -> Pca9685Result<ChannelConfig> {
        self.config.current_count = None;

        log::info!(target: &self.name, "Setting output to FULL OFF");

        match pca.set_channel_full_off(self.config.channel) {
            Ok(()) => Ok(self.config()),
            Err(error) => Err(Pca9685Error::Pca9685DriverError(error)),
        }
    }

    pub fn set_pw_ms(
        &mut self,
        pw_ms: f64,
        pca: &mut Box<dyn Pca9685Proxy>,
    ) -> Pca9685Result<ChannelConfig> {
        self.set_pwm_count(self.pw_to_count(pw_ms)?, pca)
    }

    fn pw_to_count(&self, pw_ms: f64) -> Result<u16, Pca9685Error> {
        if pw_ms < 0.0 || pw_ms > self.pca_max_pw_ms {
            return Err(Pca9685Error::PulseWidthRangeError(
                pw_ms,
                self.pca_max_pw_ms,
            ));
        }

        Ok((pw_ms / self.pca_count_length_ms) as u16)
    }

    pub fn set_pct(
        &mut self,
        pct: f64,
        pca: &mut Box<dyn Pca9685Proxy>,
    ) -> Pca9685Result<ChannelConfig> {
        if pct < 0.0 || pct > 1.0 {
            return Err(Pca9685Error::PercentOfRangeError(pct));
        }

        let (min_on_count, max_on_count) = match &self.config.custom_limits {
            Some(limits) => (limits.min_on_count, limits.max_on_count),
            None => (0, PCA_PWM_RESOLUTION),
        };
        let pwm_range_width = max_on_count - min_on_count;
        let scaled_pwm_pct = pwm_range_width as f64 * pct;
        let pwm_counts = scaled_pwm_pct as u16 + min_on_count;

        self.set_pwm_count(pwm_counts, pca)
    }

    pub fn set_pwm_count(
        &mut self,
        pwm_off_count: u16,
        pca: &mut Box<dyn Pca9685Proxy>,
    ) -> Pca9685Result<ChannelConfig> {
        let limits = match &self.config.custom_limits {
            Some(limits) => limits,
            None => &DEFAULT_CHANNEL_COUNT_LIMITS,
        };
        if !limits.is_valid(pwm_off_count) {
            return Err(Pca9685Error::CustomLimitsError(
                pwm_off_count,
                limits.clone(),
            ));
        }

        if pwm_off_count == PCA_PWM_RESOLUTION {
            self.full_on(pca)
        } else {
            match pca.set_channel_off_count(self.config.channel, pwm_off_count) {
                Ok(()) => {
                    self.config.current_count = Some(pwm_off_count);

                    log::info!(
                        target: &self.name,
                        "Setting output to {} counts ({:0.6}ms)",
                        pwm_off_count,
                        pwm_off_count as f64 * pca.single_count_duration_ms()
                    );
                    Ok(self.config())
                }
                Err(error) => Err(Pca9685Error::Pca9685DriverError(error)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{ChannelCountLimits, ChannelProxy, Pca9685Error, Pca9685Proxy, PCA_PWM_RESOLUTION};
    use pwm_pca9685::{Channel, OutputDriver};

    const TEST_OUTPUT_FREQUENCY_HZ: f64 = 200.0;
    const TEST_PCA_MAX_PW_MS: f64 = 1000.0 / TEST_OUTPUT_FREQUENCY_HZ;
    const TEST_PCA_COUNT_DURATION_MS: f64 = TEST_PCA_MAX_PW_MS / PCA_PWM_RESOLUTION as f64;

    struct MockPca9685Proxy;
    impl Pca9685Proxy for MockPca9685Proxy {
        fn max_pw_ms(&self) -> f64 {
            TEST_PCA_MAX_PW_MS
        }

        fn single_count_duration_ms(&self) -> f64 {
            TEST_PCA_COUNT_DURATION_MS
        }

        fn output_frequency_hz(&self) -> u16 {
            TEST_OUTPUT_FREQUENCY_HZ as u16
        }

        fn device(&self) -> String {
            String::from("/dev/foo")
        }

        fn address(&self) -> u8 {
            0x40
        }

        fn prescale(&self) -> u8 {
            31
        }

        fn output_type(&self) -> OutputDriver {
            OutputDriver::TotemPole
        }

        fn set_channel_off_count(
            &mut self,
            _channel: Channel,
            _off: u16,
        ) -> Result<(), pwm_pca9685::Error<linux_embedded_hal::i2cdev::linux::LinuxI2CError>>
        {
            Ok(())
        }

        fn set_channel_full_on(
            &mut self,
            _channel: Channel,
        ) -> Result<(), pwm_pca9685::Error<linux_embedded_hal::i2cdev::linux::LinuxI2CError>>
        {
            Ok(())
        }

        fn set_channel_full_off(
            &mut self,
            _channel: Channel,
        ) -> Result<(), pwm_pca9685::Error<linux_embedded_hal::i2cdev::linux::LinuxI2CError>>
        {
            Ok(())
        }
    }

    #[test]
    fn set_pwm_count() -> Result<(), Pca9685Error> {
        let mut channel = ChannelProxy::new(
            Channel::try_from(0 as u8).unwrap(),
            TEST_PCA_COUNT_DURATION_MS,
            TEST_PCA_MAX_PW_MS,
        );

        let mut mock_pca9685_proxy: Box<dyn Pca9685Proxy> = Box::new(MockPca9685Proxy {});

        // Test at min/max of range
        assert_eq!(
            channel
                .set_pwm_count(50, &mut mock_pca9685_proxy)?
                .current_count
                .unwrap(),
            50
        );

        Ok(())
    }

    #[test]
    #[should_panic(expected = "must be within the limits")]
    fn set_pwm_count_too_large() {
        let mut channel = ChannelProxy::new(
            Channel::try_from(0 as u8).unwrap(),
            TEST_PCA_COUNT_DURATION_MS,
            TEST_PCA_MAX_PW_MS,
        );

        let mut mock_pca9685_proxy: Box<dyn Pca9685Proxy> = Box::new(MockPca9685Proxy {});

        channel
            .set_pwm_count(PCA_PWM_RESOLUTION + 1, &mut mock_pca9685_proxy)
            .unwrap();
    }

    #[test]
    fn set_pw_ms() -> Result<(), Pca9685Error> {
        let mut channel = ChannelProxy::new(
            Channel::try_from(0 as u8).unwrap(),
            TEST_PCA_COUNT_DURATION_MS,
            TEST_PCA_MAX_PW_MS,
        );

        let mut mock_pca9685_proxy: Box<dyn Pca9685Proxy> = Box::new(MockPca9685Proxy {});

        // Test at min/max of range
        assert_eq!(
            channel
                .set_pw_ms(0.0, &mut mock_pca9685_proxy)?
                .current_count
                .unwrap(),
            0
        );
        assert_eq!(
            channel
                .set_pw_ms(TEST_PCA_MAX_PW_MS, &mut mock_pca9685_proxy)?
                .current_count
                .unwrap(),
            4096
        );

        // Test at percentages of range
        for pct in [0.25, 0.5, 0.75] {
            let test_pw_ms = TEST_PCA_MAX_PW_MS * pct;
            let expected_counts = (4096.0 * pct) as u16;
            assert_eq!(
                channel
                    .set_pw_ms(test_pw_ms, &mut mock_pca9685_proxy)?
                    .current_count
                    .unwrap(),
                expected_counts
            );
        }

        // Test a specific value, using formula
        for test_pw_ms in [1.0, 1.5, 2.0] {
            // Hz to to millis, so to speak
            let expected_count = 1000.0 / TEST_OUTPUT_FREQUENCY_HZ as f64;

            // Duration of each count, in millis
            let expected_count = expected_count / 4096.0;

            // Number of counts required for given test_pw_ms
            let expected_count = (test_pw_ms / expected_count) as u16;

            assert_eq!(
                channel
                    .set_pw_ms(test_pw_ms, &mut mock_pca9685_proxy)?
                    .current_count
                    .unwrap(),
                expected_count
            );
        }

        return Ok(());
    }

    #[test]
    fn set_pct() -> Result<(), Pca9685Error> {
        let mut channel = ChannelProxy::new(
            Channel::try_from(0 as u8).unwrap(),
            TEST_PCA_COUNT_DURATION_MS,
            TEST_PCA_MAX_PW_MS,
        );

        let mut mock_pca9685_proxy: Box<dyn Pca9685Proxy> = Box::new(MockPca9685Proxy {});

        // Test at percentages of range
        for pct in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let expected_counts = (4096.0 * pct) as u16;
            assert_eq!(
                channel
                    .set_pct(pct, &mut mock_pca9685_proxy)?
                    .current_count
                    .unwrap(),
                expected_counts
            );
        }

        return Ok(());
    }

    #[test]
    fn set_pct_custom_limits() -> Result<(), Pca9685Error> {
        let mut channel = ChannelProxy::new(
            Channel::try_from(0 as u8).unwrap(),
            TEST_PCA_COUNT_DURATION_MS,
            TEST_PCA_MAX_PW_MS,
        );

        let mut mock_pca9685_proxy: Box<dyn Pca9685Proxy> = Box::new(MockPca9685Proxy {});

        channel.configure_limits(&Some(ChannelCountLimits {
            min_on_count: 1000,
            max_on_count: 2000,
        }));

        // Test at percentages of range
        for pct in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let expected_counts = 1000 + (1000.0 * pct) as u16;
            assert_eq!(
                channel
                    .set_pct(pct, &mut mock_pca9685_proxy)?
                    .current_count
                    .unwrap(),
                expected_counts
            );
        }

        return Ok(());
    }

    #[test]
    #[should_panic(expected = "must be within the limits")]
    fn set_pwm_count_too_small_custom_limits() {
        let mut channel = ChannelProxy::new(
            Channel::try_from(0 as u8).unwrap(),
            TEST_PCA_COUNT_DURATION_MS,
            TEST_PCA_MAX_PW_MS,
        );

        channel.configure_limits(&Some(ChannelCountLimits {
            min_on_count: 1000,
            max_on_count: 2000,
        }));

        let mut mock_pca9685_proxy: Box<dyn Pca9685Proxy> = Box::new(MockPca9685Proxy {});

        channel.set_pwm_count(999, &mut mock_pca9685_proxy).unwrap();
    }

    #[test]
    #[should_panic(expected = "must be within the limits")]
    fn set_pwm_count_too_large_custom_limits() {
        let mut channel = ChannelProxy::new(
            Channel::try_from(0 as u8).unwrap(),
            TEST_PCA_COUNT_DURATION_MS,
            TEST_PCA_MAX_PW_MS,
        );

        channel.configure_limits(&Some(ChannelCountLimits {
            min_on_count: 1000,
            max_on_count: 2000,
        }));

        let mut mock_pca9685_proxy: Box<dyn Pca9685Proxy> = Box::new(MockPca9685Proxy {});

        channel
            .set_pwm_count(2001, &mut mock_pca9685_proxy)
            .unwrap();
    }

    #[test]
    #[should_panic(expected = "must be within the limits")]
    fn set_pw_ms_negative() {
        let mut channel = ChannelProxy::new(
            Channel::try_from(0 as u8).unwrap(),
            TEST_PCA_COUNT_DURATION_MS,
            TEST_PCA_MAX_PW_MS,
        );

        let mut mock_pca9685_proxy: Box<dyn Pca9685Proxy> = Box::new(MockPca9685Proxy {});

        channel.set_pw_ms(-1.0, &mut mock_pca9685_proxy).unwrap();
    }

    #[test]
    #[should_panic(expected = "must be within the limits")]
    fn set_pw_ms_too_large() {
        let mut channel = ChannelProxy::new(
            Channel::try_from(0 as u8).unwrap(),
            TEST_PCA_COUNT_DURATION_MS,
            TEST_PCA_MAX_PW_MS,
        );

        let mut mock_pca9685_proxy: Box<dyn Pca9685Proxy> = Box::new(MockPca9685Proxy {});

        channel
            .set_pw_ms(TEST_PCA_MAX_PW_MS + 1.0, &mut mock_pca9685_proxy)
            .unwrap();
    }
}
