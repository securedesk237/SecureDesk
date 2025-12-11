import { useState } from 'react';
import { useQuery, useMutation } from '@tanstack/react-query';
import { Check, Zap, Crown, Building2, Bitcoin, Loader2 } from 'lucide-react';
import { licenseApi, paymentApi } from '../api/client';

interface LicenseFeatures {
  tier: string;
  features: string[];
  max_users: number;
  max_devices: number;
  expires_at?: string;
}

interface PaymentOrder {
  id: string;
  tier: string;
  interval: string;
  amount: number;
  currency: string;
  status: string;
  created_at: string;
  paid_at?: string;
}

const plans = [
  {
    id: 'free',
    name: 'Free',
    monthlyPrice: 0,
    yearlyPrice: 0,
    icon: Zap,
    color: 'gray',
    users: 1,
    devices: 3,
    features: [
      'Remote desktop access',
      'File transfer',
      'Basic encryption',
      'Community support',
    ],
  },
  {
    id: 'basic',
    name: 'Basic',
    monthlyPrice: 9.99,
    yearlyPrice: 99.99,
    icon: Crown,
    color: 'blue',
    users: 1,
    devices: 20,
    popular: true,
    features: [
      'Everything in Free',
      '2FA Authentication',
      'Web console access',
      'Address book',
      'Audit logging',
      'Access control',
      'Priority support',
    ],
  },
  {
    id: 'pro',
    name: 'Pro',
    monthlyPrice: 29.99,
    yearlyPrice: 299.99,
    icon: Building2,
    color: 'purple',
    users: 10,
    devices: 100,
    features: [
      'Everything in Basic',
      'OIDC/SSO integration',
      'LDAP directory sync',
      'Custom branded client',
      'WebSocket API access',
      'Dedicated support',
      'SLA guarantee',
    ],
  },
];

export default function Subscription() {
  const [billingInterval, setBillingInterval] = useState<'monthly' | 'yearly'>('monthly');
  const [selectedPlan, setSelectedPlan] = useState<string | null>(null);

  const { data: license } = useQuery({
    queryKey: ['license-features'],
    queryFn: () => licenseApi.getFeatures().then((r) => r.data as LicenseFeatures),
  });

  const { data: orders } = useQuery({
    queryKey: ['payment-orders'],
    queryFn: () => paymentApi.getOrders().then((r) => r.data as PaymentOrder[]),
  });

  const createInvoiceMutation = useMutation({
    mutationFn: ({ tier, interval }: { tier: string; interval: 'monthly' | 'yearly' }) =>
      paymentApi.createInvoice(tier, interval),
    onSuccess: (response) => {
      window.location.href = response.data.checkout_url;
    },
    onError: () => {
      alert('Failed to create payment invoice. Please try again.');
    },
  });

  const currentTier = license?.tier || 'free';

  const handleUpgrade = (planId: string) => {
    setSelectedPlan(planId);
    createInvoiceMutation.mutate({ tier: planId, interval: billingInterval });
  };

  const urlParams = new URLSearchParams(window.location.search);
  const paymentStatus = urlParams.get('payment');

  return (
    <div>
      <div className="mb-8">
        <h1 className="text-2xl font-bold text-gray-900">Subscription</h1>
        <p className="text-gray-600">
          Choose the plan that's right for you. Pay with cryptocurrency.
        </p>
      </div>

      {paymentStatus === 'success' && (
        <div className="bg-green-50 border border-green-200 rounded-xl p-4 mb-6">
          <div className="flex items-center gap-3">
            <Check className="w-5 h-5 text-green-600" />
            <div>
              <p className="font-medium text-green-800">Payment Successful!</p>
              <p className="text-sm text-green-600">
                Your subscription has been activated. Thank you for your purchase!
              </p>
            </div>
          </div>
        </div>
      )}

      {/* Current Plan */}
      <div className="bg-gradient-to-r from-blue-600 to-indigo-600 rounded-xl p-6 text-white mb-8">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-blue-200 text-sm">Current Plan</p>
            <h2 className="text-2xl font-bold capitalize">{currentTier}</h2>
            <p className="text-blue-100 mt-1">
              {license?.max_devices || 3} devices • {license?.max_users || 1} user
            </p>
          </div>
          {license?.expires_at && (
            <div className="text-right">
              <p className="text-blue-200 text-sm">Renews</p>
              <p className="font-medium">
                {new Date(license.expires_at).toLocaleDateString()}
              </p>
            </div>
          )}
        </div>
      </div>

      {/* Billing Toggle */}
      <div className="flex items-center justify-center gap-4 mb-8">
        <span className={`text-sm ${billingInterval === 'monthly' ? 'text-gray-900 font-medium' : 'text-gray-500'}`}>
          Monthly
        </span>
        <button
          onClick={() => setBillingInterval(billingInterval === 'monthly' ? 'yearly' : 'monthly')}
          className={`relative w-14 h-7 rounded-full transition-colors ${
            billingInterval === 'yearly' ? 'bg-blue-600' : 'bg-gray-300'
          }`}
        >
          <span
            className={`absolute top-1 w-5 h-5 bg-white rounded-full transition-transform ${
              billingInterval === 'yearly' ? 'translate-x-8' : 'translate-x-1'
            }`}
          />
        </button>
        <span className={`text-sm ${billingInterval === 'yearly' ? 'text-gray-900 font-medium' : 'text-gray-500'}`}>
          Yearly
          <span className="ml-1 text-green-600 text-xs font-medium">Save 17%</span>
        </span>
      </div>

      {/* Crypto Payment Notice */}
      <div className="bg-orange-50 border border-orange-200 rounded-xl p-4 mb-8">
        <div className="flex items-center gap-3">
          <Bitcoin className="w-6 h-6 text-orange-600" />
          <div>
            <p className="font-medium text-orange-800">Cryptocurrency Payments Only</p>
            <p className="text-sm text-orange-600">
              We accept Bitcoin (BTC), Litecoin (LTC), and Monero (XMR) for maximum privacy.
            </p>
          </div>
        </div>
      </div>

      {/* Plans Grid */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
        {plans.map((plan) => {
          const isCurrent = plan.id === currentTier;
          const isUpgrade =
            (currentTier === 'free' && (plan.id === 'basic' || plan.id === 'pro')) ||
            (currentTier === 'basic' && plan.id === 'pro');
          const price = billingInterval === 'monthly' ? plan.monthlyPrice : plan.yearlyPrice;
          const isLoading = createInvoiceMutation.isPending && selectedPlan === plan.id;

          return (
            <div
              key={plan.id}
              className={`bg-white rounded-xl shadow-sm border-2 ${
                plan.popular
                  ? 'border-blue-500 relative'
                  : isCurrent
                  ? 'border-green-500'
                  : 'border-transparent'
              }`}
            >
              {plan.popular && (
                <div className="absolute -top-3 left-1/2 -translate-x-1/2">
                  <span className="bg-blue-500 text-white text-xs px-3 py-1 rounded-full">
                    Most Popular
                  </span>
                </div>
              )}

              <div className="p-6">
                <div className="flex items-center gap-3 mb-4">
                  <div
                    className={`p-2 rounded-lg ${
                      plan.color === 'gray'
                        ? 'bg-gray-100'
                        : plan.color === 'blue'
                        ? 'bg-blue-100'
                        : 'bg-purple-100'
                    }`}
                  >
                    <plan.icon
                      className={`w-5 h-5 ${
                        plan.color === 'gray'
                          ? 'text-gray-600'
                          : plan.color === 'blue'
                          ? 'text-blue-600'
                          : 'text-purple-600'
                      }`}
                    />
                  </div>
                  <h3 className="text-lg font-semibold text-gray-900">{plan.name}</h3>
                </div>

                <div className="mb-6">
                  <span className="text-3xl font-bold text-gray-900">
                    ${price.toFixed(2)}
                  </span>
                  <span className="text-gray-500">
                    {plan.id !== 'free' && (billingInterval === 'monthly' ? '/month' : '/year')}
                  </span>
                </div>

                <div className="text-sm text-gray-600 mb-4">
                  {plan.users} user{plan.users > 1 ? 's' : ''} • {plan.devices} devices
                </div>

                <ul className="space-y-3 mb-6">
                  {plan.features.map((feature) => (
                    <li key={feature} className="flex items-start gap-2 text-sm">
                      <Check className="w-4 h-4 text-green-500 mt-0.5 flex-shrink-0" />
                      <span className="text-gray-600">{feature}</span>
                    </li>
                  ))}
                </ul>

                {isCurrent ? (
                  <button
                    disabled
                    className="w-full py-2 rounded-lg bg-green-100 text-green-700 font-medium"
                  >
                    Current Plan
                  </button>
                ) : isUpgrade ? (
                  <button
                    onClick={() => handleUpgrade(plan.id)}
                    disabled={isLoading}
                    className={`w-full py-2 rounded-lg font-medium transition-colors flex items-center justify-center gap-2 ${
                      plan.popular
                        ? 'bg-blue-600 text-white hover:bg-blue-700'
                        : 'bg-gray-900 text-white hover:bg-gray-800'
                    } disabled:opacity-50`}
                  >
                    {isLoading ? (
                      <>
                        <Loader2 className="w-4 h-4 animate-spin" />
                        Creating Invoice...
                      </>
                    ) : (
                      <>
                        <Bitcoin className="w-4 h-4" />
                        Pay with Crypto
                      </>
                    )}
                  </button>
                ) : (
                  <button
                    disabled
                    className="w-full py-2 rounded-lg bg-gray-100 text-gray-400 font-medium"
                  >
                    {plan.id === 'free' ? 'Free Forever' : 'Downgrade'}
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </div>

      {/* Payment History */}
      {orders && orders.length > 0 && (
        <div className="mt-12">
          <h2 className="text-lg font-semibold text-gray-900 mb-4">Payment History</h2>
          <div className="bg-white rounded-xl shadow-sm overflow-hidden">
            <table className="w-full">
              <thead className="bg-gray-50">
                <tr>
                  <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase">Date</th>
                  <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase">Plan</th>
                  <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase">Amount</th>
                  <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase">Status</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-200">
                {orders.map((order) => (
                  <tr key={order.id}>
                    <td className="px-4 py-3 text-sm text-gray-900">
                      {new Date(order.created_at).toLocaleDateString()}
                    </td>
                    <td className="px-4 py-3 text-sm text-gray-900 capitalize">
                      {order.tier} ({order.interval})
                    </td>
                    <td className="px-4 py-3 text-sm text-gray-900">
                      ${order.amount.toFixed(2)} {order.currency}
                    </td>
                    <td className="px-4 py-3">
                      <span
                        className={`inline-flex items-center px-2 py-1 rounded-full text-xs font-medium ${
                          order.status === 'completed'
                            ? 'bg-green-100 text-green-800'
                            : order.status === 'pending'
                            ? 'bg-yellow-100 text-yellow-800'
                            : 'bg-red-100 text-red-800'
                        }`}
                      >
                        {order.status}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* FAQ */}
      <div className="mt-12">
        <h2 className="text-lg font-semibold text-gray-900 mb-4">
          Frequently Asked Questions
        </h2>
        <div className="space-y-4">
          <div className="bg-white rounded-lg p-4">
            <h3 className="font-medium text-gray-900 mb-2">
              How do crypto payments work?
            </h3>
            <p className="text-sm text-gray-600">
              After clicking "Pay with Crypto", you'll be redirected to our secure payment page where
              you can pay with Bitcoin, Litecoin, or Monero. Your license activates automatically once
              the payment is confirmed on the blockchain.
            </p>
          </div>
          <div className="bg-white rounded-lg p-4">
            <h3 className="font-medium text-gray-900 mb-2">
              Can I change my plan anytime?
            </h3>
            <p className="text-sm text-gray-600">
              Yes, you can upgrade your plan at any time. When you upgrade, your new plan activates
              immediately and your remaining time is prorated.
            </p>
          </div>
          <div className="bg-white rounded-lg p-4">
            <h3 className="font-medium text-gray-900 mb-2">
              What happens if I exceed my device limit?
            </h3>
            <p className="text-sm text-gray-600">
              You won't be able to add new devices until you remove existing ones or upgrade
              your plan. Existing devices will continue to work.
            </p>
          </div>
          <div className="bg-white rounded-lg p-4">
            <h3 className="font-medium text-gray-900 mb-2">
              Do you offer refunds?
            </h3>
            <p className="text-sm text-gray-600">
              Due to the nature of cryptocurrency payments, we cannot offer refunds. However,
              you can try the free tier before upgrading to ensure SecureDesk meets your needs.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
