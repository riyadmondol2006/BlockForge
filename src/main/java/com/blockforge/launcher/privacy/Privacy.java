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

import com.blockforge.launcher.App;

/**
 * The one place that decides whether anything may be sent to a server about how BlockForge is used. Everything that
 * reports usage asks here first, and the answer is "no" unless the person turned it on.
 */
public final class Privacy {
    private Privacy() {
    }

    /** anonymous usage statistics (which screens are used, which instances are launched) */
    public static boolean analyticsAllowed() {
        return App.settings != null && App.settings.enableAnalytics && !App.disableAnalytics;
    }

    /** counts of installs and time played for the packs that have a public page */
    public static boolean usageLogsAllowed() {
        return App.settings != null && App.settings.enableLogs;
    }

    /** crash reports to a third party service */
    public static boolean errorReportingAllowed() {
        return !App.disableErrorReporting;
    }
}
