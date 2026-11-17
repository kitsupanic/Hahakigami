export interface AppToast {
  id: number;
  message: string;
  action?: { label: string; run: () => void };
}
