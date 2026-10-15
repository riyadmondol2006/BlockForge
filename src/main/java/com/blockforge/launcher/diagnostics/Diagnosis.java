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
package com.blockforge.launcher.diagnostics;

import java.util.Objects;

/**
 * What most likely went wrong, in plain words. Every diagnosis has a stable code that docs and support can refer to.
 */
public final class Diagnosis {
    public final String code;
    public final String title;
    public final String explanation;
    public final String fix;
    /** the part of the log that led to this diagnosis */
    public final String evidence;

    public Diagnosis(String code, String title, String explanation, String fix, String evidence) {
        this.code = Objects.requireNonNull(code);
        this.title = Objects.requireNonNull(title);
        this.explanation = Objects.requireNonNull(explanation);
        this.fix = Objects.requireNonNull(fix);
        this.evidence = evidence == null ? "" : evidence;
    }

    @Override
    public String toString() {
        return code + ": " + title;
    }
}
