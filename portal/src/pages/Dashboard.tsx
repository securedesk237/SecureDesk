import { useQuery } from '@tanstack/react-query';
import { Monitor, Key, Shield, Download } from 'lucide-react';
import { useAuthStore } from '../store/auth';
import { devicesApi, licenseApi } from '../api/client';

interface LicenseFeatures {
  tier: string;
  features: string[];
  max_users: number;
  max_devices: number;
  expires_at?: string;
}

interface Device {
  id: string;
  device_id: string;
  name: string;
  is_online: boolean;
}

export default function Dashboard() {
  const user = useAuthStore((s) => s.user);

  const { data: license } = useQuery({
    queryKey: ['license-features'],
    queryFn: () => licenseApi.getFeatures().then((r) => r.data as LicenseFeatures),
  });

  const { data: devices } = useQuery({
    queryKey: ['devices'],
    queryFn: () => devicesApi.list().then((r) => r.data as Device[]),
  });

  const onlineDevices = devices?.filter((d) => d.is_online).length || 0;
  const totalDevices = devices?.length || 0;

  const tierColors: Record<string, string> = {
    free: 'bg-gray-100 text-gray-700',
    basic: 'bg-blue-100 text-blue-700',
    pro: 'bg-purple-100 text-purple-700',
  };

  return (
    <div>
      <div className="mb-8">
        <h1 className="text-2xl font-bold text-gray-900">
          Welcome back, {user?.name}
        </h1>
        <p className="text-gray-600">
          Manage your devices and subscription from here.
        </p>
      </div>

      {/* Quick Stats */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6 mb-8">
        <div className="bg-white rounded-xl p-6 shadow-sm">
          <div className="flex items-center gap-4">
            <div className="p-3 bg-blue-100 rounded-lg">
              <Monitor className="w-6 h-6 text-blue-600" />
            </div>
            <div>
              <p className="text-sm text-gray-500">Devices</p>
              <p className="text-2xl font-bold text-gray-900">
                {totalDevices} / {license?.max_devices || 3}
              </p>
              <p className="text-xs text-green-600">{onlineDevices} online</p>
            </div>
          </div>
        </div>

        <div className="bg-white rounded-xl p-6 shadow-sm">
          <div className="flex items-center gap-4">
            <div className="p-3 bg-purple-100 rounded-lg">
              <Key className="w-6 h-6 text-purple-600" />
            </div>
            <div>
              <p className="text-sm text-gray-500">Subscription</p>
              <p className="text-2xl font-bold text-gray-900 flex items-center gap-2">
                <span
                  className={`px-2 py-1 text-sm rounded-full ${
                    tierColors[license?.tier || 'free']
                  }`}
                >
                  {license?.tier?.toUpperCase() || 'FREE'}
                </span>
              </p>
            </div>
          </div>
        </div>

        <div className="bg-white rounded-xl p-6 shadow-sm">
          <div className="flex items-center gap-4">
            <div className="p-3 bg-green-100 rounded-lg">
              <Shield className="w-6 h-6 text-green-600" />
            </div>
            <div>
              <p className="text-sm text-gray-500">2FA Status</p>
              <p className="text-2xl font-bold text-gray-900">
                {user?.two_factor_enabled ? 'Enabled' : 'Disabled'}
              </p>
            </div>
          </div>
        </div>
      </div>

      {/* Download Section */}
      <div className="bg-gradient-to-r from-blue-600 to-indigo-600 rounded-xl p-8 text-white mb-8">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-xl font-bold mb-2">Download SecureDesk</h2>
            <p className="text-blue-100 mb-4">
              Get the desktop application to connect to your devices remotely.
            </p>
            <div className="flex gap-3">
              <button className="flex items-center gap-2 bg-white text-blue-600 px-4 py-2 rounded-lg hover:bg-blue-50 transition-colors font-medium">
                <Download className="w-4 h-4" />
                Windows
              </button>
              <button className="flex items-center gap-2 bg-white/20 text-white px-4 py-2 rounded-lg hover:bg-white/30 transition-colors font-medium">
                <Download className="w-4 h-4" />
                macOS
              </button>
              <button className="flex items-center gap-2 bg-white/20 text-white px-4 py-2 rounded-lg hover:bg-white/30 transition-colors font-medium">
                <Download className="w-4 h-4" />
                Linux
              </button>
            </div>
          </div>
          <Monitor className="w-24 h-24 text-white/20" />
        </div>
      </div>

      {/* Features */}
      <div className="bg-white rounded-xl p-6 shadow-sm">
        <h2 className="text-lg font-semibold text-gray-900 mb-4">
          Your Features
        </h2>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          {license?.features?.map((feature) => (
            <div
              key={feature}
              className="flex items-center gap-2 text-sm text-gray-600"
            >
              <div className="w-2 h-2 bg-green-500 rounded-full"></div>
              {formatFeature(feature)}
            </div>
          ))}
        </div>

        {license?.tier === 'free' && (
          <div className="mt-6 pt-6 border-t">
            <p className="text-sm text-gray-500 mb-3">
              Upgrade to unlock more features and devices
            </p>
            <a
              href="/subscription"
              className="inline-block bg-blue-600 text-white px-4 py-2 rounded-lg hover:bg-blue-700 transition-colors font-medium text-sm"
            >
              View Plans
            </a>
          </div>
        )}
      </div>
    </div>
  );
}

function formatFeature(feature: string): string {
  return feature
    .split('_')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}
