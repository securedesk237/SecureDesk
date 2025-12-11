import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Monitor, Wifi, WifiOff, Trash2, Edit2, X, Loader2 } from 'lucide-react';
import { formatDistanceToNow } from 'date-fns';
import { devicesApi, licenseApi } from '../api/client';

interface Device {
  id: string;
  device_id: string;
  name: string;
  os: string;
  os_version: string;
  last_ip: string;
  last_seen?: string;
  is_online: boolean;
  created_at: string;
}

interface LicenseFeatures {
  max_devices: number;
}

export default function Devices() {
  const queryClient = useQueryClient();
  const [editingDevice, setEditingDevice] = useState<Device | null>(null);

  const { data: devices, isLoading } = useQuery({
    queryKey: ['devices'],
    queryFn: () => devicesApi.list().then((r) => r.data as Device[]),
  });

  const { data: license } = useQuery({
    queryKey: ['license-features'],
    queryFn: () => licenseApi.getFeatures().then((r) => r.data as LicenseFeatures),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => devicesApi.delete(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['devices'] });
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, name }: { id: string; name: string }) =>
      devicesApi.update(id, name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['devices'] });
      setEditingDevice(null);
    },
  });

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600"></div>
      </div>
    );
  }

  const maxDevices = license?.max_devices || 3;
  const currentDevices = devices?.length || 0;

  return (
    <div>
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold text-gray-900">Devices</h1>
          <p className="text-gray-600">
            {currentDevices} of {maxDevices} devices registered
          </p>
        </div>
      </div>

      {/* Device Limit Warning */}
      {currentDevices >= maxDevices && (
        <div className="mb-6 p-4 bg-amber-50 border border-amber-200 rounded-lg">
          <p className="text-amber-800 text-sm">
            You've reached your device limit. Remove a device or upgrade your plan to add more.
          </p>
        </div>
      )}

      {/* Device List */}
      <div className="space-y-4">
        {devices?.map((device) => (
          <div
            key={device.id}
            className="bg-white rounded-xl p-6 shadow-sm flex items-center justify-between"
          >
            <div className="flex items-center gap-4">
              <div
                className={`p-3 rounded-lg ${
                  device.is_online ? 'bg-green-100' : 'bg-gray-100'
                }`}
              >
                <Monitor
                  className={`w-6 h-6 ${
                    device.is_online ? 'text-green-600' : 'text-gray-400'
                  }`}
                />
              </div>
              <div>
                <div className="flex items-center gap-2">
                  <h3 className="font-medium text-gray-900">
                    {device.name || 'Unnamed Device'}
                  </h3>
                  {device.is_online ? (
                    <span className="flex items-center gap-1 text-xs text-green-600">
                      <Wifi className="w-3 h-3" /> Online
                    </span>
                  ) : (
                    <span className="flex items-center gap-1 text-xs text-gray-400">
                      <WifiOff className="w-3 h-3" /> Offline
                    </span>
                  )}
                </div>
                <div className="flex items-center gap-4 mt-1">
                  <code className="text-sm text-gray-500 bg-gray-100 px-2 py-0.5 rounded">
                    {device.device_id}
                  </code>
                  <span className="text-sm text-gray-400">
                    {device.os} {device.os_version}
                  </span>
                </div>
                {device.last_seen && (
                  <p className="text-xs text-gray-400 mt-1">
                    Last seen{' '}
                    {formatDistanceToNow(new Date(device.last_seen), {
                      addSuffix: true,
                    })}
                  </p>
                )}
              </div>
            </div>

            <div className="flex items-center gap-2">
              <button
                onClick={() => setEditingDevice(device)}
                className="p-2 text-gray-400 hover:text-blue-600 transition-colors"
                title="Rename"
              >
                <Edit2 className="w-4 h-4" />
              </button>
              <button
                onClick={() => {
                  if (confirm('Remove this device?')) {
                    deleteMutation.mutate(device.id);
                  }
                }}
                className="p-2 text-gray-400 hover:text-red-600 transition-colors"
                title="Remove"
              >
                <Trash2 className="w-4 h-4" />
              </button>
            </div>
          </div>
        ))}
      </div>

      {devices?.length === 0 && (
        <div className="text-center py-12 bg-white rounded-xl">
          <Monitor className="w-12 h-12 text-gray-300 mx-auto mb-4" />
          <h3 className="text-lg font-medium text-gray-900 mb-2">No devices yet</h3>
          <p className="text-gray-500">
            Download and install SecureDesk on your devices to get started.
          </p>
        </div>
      )}

      {/* Edit Modal */}
      {editingDevice && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white rounded-xl shadow-xl w-full max-w-md mx-4">
            <div className="flex items-center justify-between p-4 border-b">
              <h2 className="text-lg font-semibold">Rename Device</h2>
              <button
                onClick={() => setEditingDevice(null)}
                className="text-gray-400 hover:text-gray-600"
              >
                <X className="w-5 h-5" />
              </button>
            </div>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                const form = e.currentTarget;
                const formData = new FormData(form);
                updateMutation.mutate({
                  id: editingDevice.id,
                  name: formData.get('name') as string,
                });
              }}
              className="p-4 space-y-4"
            >
              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  Device Name
                </label>
                <input
                  name="name"
                  defaultValue={editingDevice.name}
                  className="w-full px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500"
                  placeholder="My Computer"
                />
              </div>
              <div className="flex justify-end gap-3 pt-4">
                <button
                  type="button"
                  onClick={() => setEditingDevice(null)}
                  className="px-4 py-2 text-gray-700 hover:bg-gray-100 rounded-lg"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  disabled={updateMutation.isPending}
                  className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 flex items-center gap-2"
                >
                  {updateMutation.isPending && (
                    <Loader2 className="w-4 h-4 animate-spin" />
                  )}
                  Save
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  );
}
