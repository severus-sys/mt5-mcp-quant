#property copyright "MT5-MCP-Quant"
#property version   "1.00"
#property strict

input double Lots = 0.01;
input long Magic = 5500001;

bool order_sent = false;

int OnInit()
{
   return(INIT_SUCCEEDED);
}

void OnTick()
{
   if(order_sent || PositionsTotal() > 0)
      return;

   MqlTradeRequest request = {};
   MqlTradeResult result = {};
   request.action = TRADE_ACTION_DEAL;
   request.symbol = _Symbol;
   request.volume = Lots;
   request.type = ORDER_TYPE_BUY;
   request.price = SymbolInfoDouble(_Symbol, SYMBOL_ASK);
   request.deviation = 20;
   request.magic = Magic;
   request.comment = "mt5-mcp-quant-windows-smoke";

   long filling = SymbolInfoInteger(_Symbol, SYMBOL_FILLING_MODE);
   if((filling & SYMBOL_FILLING_FOK) == SYMBOL_FILLING_FOK)
      request.type_filling = ORDER_FILLING_FOK;
   else if((filling & SYMBOL_FILLING_IOC) == SYMBOL_FILLING_IOC)
      request.type_filling = ORDER_FILLING_IOC;
   else
      request.type_filling = ORDER_FILLING_RETURN;

   if(OrderSend(request, result))
      order_sent = true;
}
