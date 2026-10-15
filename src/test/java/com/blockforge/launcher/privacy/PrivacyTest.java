/*
 * BlockForge - a fork of ATLauncher (https://github.com/ATLauncher/ATLauncher)
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program. If not, see <http://www.gnu.org/licenses/>.
 */
package com.blockforge.launcher.privacy;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.HashMap;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import com.blockforge.launcher.App;
import com.blockforge.launcher.data.Settings;
import com.blockforge.launcher.network.Analytics;
import com.blockforge.launcher.network.ErrorReporting;
import com.blockforge.launcher.network.analytics.AnalyticsEvent;

/** Nothing about how BlockForge is used may leave the computer unless the person turned it on. */
class PrivacyTest {
    private Settings previousSettings;

    @BeforeEach
    void freshSettings() {
        previousSettings = App.settings;
        App.settings = new Settings();
        Analytics.sendAllStoredEvents(false);
    }

    @AfterEach
    void restore() {
        App.settings = previousSettings;
    }

    @Test
    void everythingIsOffByDefault() {
        assertFalse(App.settings.enableAnalytics);
        assertFalse(App.settings.enableLogs);
        assertFalse(App.settings.enableConsole);
        assertFalse(Privacy.analyticsAllowed());
        assertFalse(Privacy.usageLogsAllowed());
        assertFalse(Privacy.errorReportingAllowed());
    }

    @Test
    void usageEventsAreNotEvenCollectedWhileOff() {
        Analytics.trackEvent(new AnalyticsEvent("something_happened", new HashMap<>()));
        assertEquals(0, Analytics.pendingEventCount());
    }

    @Test
    void turningStatisticsOnCollectsAndTurningThemOffDiscards() {
        App.settings.enableAnalytics = true;
        Analytics.trackEvent(new AnalyticsEvent("something_happened", new HashMap<>()));
        assertEquals(1, Analytics.pendingEventCount());

        // changed their mind before anything was sent: what was collected is thrown away, not sent
        App.settings.enableAnalytics = false;
        Analytics.sendAllStoredEvents(false);
        assertEquals(0, Analytics.pendingEventCount());
    }

    @Test
    void errorReportingCannotBeSwitchedOnByTheServer() {
        // the launcher's remote config asks for it to start, which must be ignored while it is off
        ErrorReporting.enable();
        assertFalse(ErrorReporting.sentryInitialised);
    }

    @Test
    void anExplicitFlagTurnsErrorReportingOn() {
        boolean before = App.disableErrorReporting;
        try {
            App.disableErrorReporting = false;
            assertTrue(Privacy.errorReportingAllowed());
        } finally {
            App.disableErrorReporting = before;
        }
    }
}
