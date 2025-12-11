import axios from 'axios';
import { useAuthStore } from '../store/auth';

const api = axios.create({
  baseURL: '/api',
  headers: {
    'Content-Type': 'application/json',
  },
});

api.interceptors.request.use((config) => {
  const token = useAuthStore.getState().accessToken;
  if (token) {
    config.headers.Authorization = `Bearer ${token}`;
  }
  return config;
});

api.interceptors.response.use(
  (response) => response,
  async (error) => {
    const originalRequest = error.config;

    if (error.response?.status === 401 && !originalRequest._retry) {
      originalRequest._retry = true;

      const refreshToken = useAuthStore.getState().refreshToken;
      if (refreshToken) {
        try {
          const { data } = await axios.post('/api/auth/refresh', {
            refresh_token: refreshToken,
          });

          useAuthStore.setState({
            accessToken: data.access_token,
            refreshToken: data.refresh_token,
          });

          originalRequest.headers.Authorization = `Bearer ${data.access_token}`;
          return api(originalRequest);
        } catch {
          useAuthStore.getState().logout();
          window.location.href = '/login';
        }
      }
    }

    return Promise.reject(error);
  }
);

export default api;

// Auth API
export const authApi = {
  register: (email: string, password: string, name: string, organizationName?: string) =>
    api.post('/auth/register', { email, password, name, organization_name: organizationName }),
  login: (email: string, password: string, totpCode?: string) =>
    api.post('/auth/login', { email, password, totp_code: totpCode }),
  logout: () => api.post('/auth/logout'),
  me: () => api.get('/auth/me'),
  changePassword: (currentPassword: string, newPassword: string) =>
    api.post('/auth/password/change', { current_password: currentPassword, new_password: newPassword }),
  enable2FA: () => api.post('/auth/2fa/enable'),
  verify2FA: (code: string) => api.post('/auth/2fa/verify', { code }),
};

// Devices API
export const devicesApi = {
  list: () => api.get('/devices'),
  get: (id: string) => api.get(`/devices/${id}`),
  register: (deviceId: string, name?: string, os?: string, osVersion?: string) =>
    api.post('/devices/register', { device_id: deviceId, name, os, os_version: osVersion }),
  update: (id: string, name: string) => api.put(`/devices/${id}`, { name }),
  delete: (id: string) => api.delete(`/devices/${id}`),
};

// License API
export const licenseApi = {
  getFeatures: () => api.get('/license/features'),
};

// Payment API
export const paymentApi = {
  getPricing: () => api.get('/pricing'),
  createInvoice: (tier: string, interval: 'monthly' | 'yearly') =>
    api.post('/payments/create-invoice', { tier, interval }),
  getOrders: () => api.get('/payments/orders'),
};
