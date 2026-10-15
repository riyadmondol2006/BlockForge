/*
 * ATLauncher - https://github.com/ATLauncher/ATLauncher
 * Copyright (C) 2013-2022 ATLauncher
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
package com.blockforge.launcher.data.json;

import java.io.File;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.util.List;
import java.util.Map;

import com.blockforge.launcher.annot.Json;
import com.blockforge.launcher.data.minecraft.loaders.LoaderVersion;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.workers.InstanceInstaller;

@Json
public class Loader {
    public String type;
    public boolean choose = false;
    public Map<String, Object> metadata;
    public String className;
    public String chooseClassName;
    public String chooseMethod;

    public String getType() {
        return this.type;
    }

    public boolean canChoose() {
        return this.choose;
    }

    public Map<String, Object> getMetadata() {
        return this.metadata;
    }

    public String getClassName() {
        return this.className;
    }

    public String getChooseClassName() {
        return this.chooseClassName;
    }

    public String getChooseMethod() {
        return this.chooseMethod;
    }

    public com.blockforge.launcher.data.minecraft.loaders.Loader getLoader(File tempDir, InstanceInstaller instanceInstaller,
            LoaderVersion loaderVersion) throws Exception {
        com.blockforge.launcher.data.minecraft.loaders.Loader instance = Class
                .forName(this.className.replace("com.blockforge.launcher.data.loaders", "com.blockforge.launcher.data.minecraft.loaders"))
                .asSubclass(com.blockforge.launcher.data.minecraft.loaders.Loader.class)
                .getDeclaredConstructor().newInstance();

        instance.set(this.metadata, tempDir, instanceInstaller, loaderVersion);

        return instance;
    }

    @SuppressWarnings("unchecked")
    public List<LoaderVersion> getChoosableVersions(String minecraft) {
        try {
            Method method = Class
                    .forName(this.chooseClassName.replace("com.blockforge.launcher.data.loaders",
                            "com.blockforge.launcher.data.minecraft.loaders"))
                    .getDeclaredMethod(this.chooseMethod, String.class);

            return (List<LoaderVersion>) method.invoke(null, minecraft);
        } catch (NoSuchMethodException | SecurityException | ClassNotFoundException | IllegalAccessException
                | IllegalArgumentException | InvocationTargetException e) {
            LogManager.logStackTrace(e);
        }

        return null;
    }
}
